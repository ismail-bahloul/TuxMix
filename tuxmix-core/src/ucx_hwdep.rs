// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Ismaïl Bahloul

//! The Fireface UCX's DSP over the ALSA hwdep node of `snd-fireface-ucx`
//! (realbogart/fireface-ucx-linux), `/dev/snd/hwC<card>D<dev>`.
//!
//! That driver owns the USB side: it streams the audio, polls the unit,
//! keeps a table of every register the unit has reported or the host has
//! written, paces DSP writes to the unit and refuses the flash registers.
//! Its hwdep node exposes all of it to user space without root:
//!
//! - `write()`: DSP words, host-endian `u32` `reg << 16 | value` (the
//!   driver adds the parity), at most [`MAX_WORDS`] per call;
//! - `read()`: the register table, the same word layout;
//! - ioctl `METERS`: one read of each meter bank (`latter::meters`);
//! - ioctl `STATUS`: clock and sync state, firmware version, serial.
//!
//! The ABI is the driver's `include/ucx_hwdep.h`, licensed
//! `(GPL-2.0 WITH Linux-syscall-note) OR MIT`. This module mirrors it; a
//! test pins the ioctl numbers so a change on either side shows.
//!
//! This is the transport only: no `RmeDevice` uses it yet. The reading
//! half can be tried on a unit with `cargo run -p tuxmix-core --example
//! ucx-probe`, which never writes.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::latter::{self, meters};

/// Name the driver gives its hwdep device (`snd_hwdep_new`).
pub const HWDEP_NAME: &str = "Fireface UCX DSP";
/// Most words in one `write()`.
pub const MAX_WORDS: usize = 63;
/// Bytes to read the whole register table in one call.
pub const REGS_BYTES: usize = 0x8000 * 4;
const METER_BANKS: usize = 5;
const METER_BYTES: usize = 512;
/// `struct ucx_hwdep_meters`: per bank a `u32` length, then the data.
const METERS_STRUCT_BYTES: usize = METER_BANKS * (4 + METER_BYTES);
/// `struct ucx_hwdep_status`: four `u32`.
const STATUS_STRUCT_BYTES: usize = 16;

/// Linux `_IOR(type, nr, size)`.
const fn ior(ty: u8, nr: u8, size: usize) -> u64 {
    (2u64 << 30) | ((size as u64) << 16) | ((ty as u64) << 8) | nr as u64
}

/// `UCX_HWDEP_METERS`.
pub const IOCTL_METERS: u64 = ior(b'U', 0x01, METERS_STRUCT_BYTES);
/// `UCX_HWDEP_STATUS`.
pub const IOCTL_STATUS: u64 = ior(b'U', 0x02, STATUS_STRUCT_BYTES);

/// Pseudo-registers the driver adds to its table (not the unit's): the
/// "EQ+D for record" setting and the loopback slot of each output, which
/// the unit reports nowhere. Read-only through the table.
pub const REG_EQD_RECORD: u16 = 0x7f10;
/// Loopback of output `n` (0 = Line 1 ... 17 = ADAT 8) is `REG_LOOPBACK + n`.
pub const REG_LOOPBACK: u16 = 0x7f20;

fn is_driver_register(reg: u16) -> bool {
    reg == REG_EQD_RECORD || (REG_LOOPBACK..REG_LOOPBACK + 18).contains(&reg)
}

/// What the node is opened as: a file in production, a fake in tests.
pub trait HwdepIo {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize>;
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    /// An `_IOR` ioctl that fills `buf`.
    fn ioctl_read(&mut self, request: u64, buf: &mut [u8]) -> io::Result<()>;
}

/// The hwdep node as a file.
pub struct FileIo(File);

impl HwdepIo for FileIo {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }

    fn ioctl_read(&mut self, request: u64, buf: &mut [u8]) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        // SAFETY: the request is one of this module's `_IOR` numbers, whose
        // size field matches `buf`'s length (checked by the callers), so
        // the kernel writes inside `buf`.
        let r = unsafe { libc::ioctl(self.0.as_raw_fd(), request as _, buf.as_mut_ptr()) };
        if r < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

/// The unit's status words (`struct ucx_hwdep_status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    /// Request 0x11: clock, lock and sync state.
    pub status: u32,
    /// Request 0x1c: firmware version.
    pub version: u32,
    /// Requests 0x1e, 0x1f.
    pub serial: [u32; 2],
}

impl Status {
    /// The active sample-rate code (`UCX_STATUS_RATE`), `None` when the
    /// unit reports none (0xf).
    pub fn rate_code(&self) -> Option<u8> {
        let r = (self.status >> 24 & 0xf) as u8;
        (r != 0xf).then_some(r)
    }

    /// The clock source (`UCX_STATUS_CLOCK`).
    pub fn clock(&self) -> &'static str {
        match self.status >> 9 & 0x7 {
            7 => "Internal",
            3 => "Word Clock",
            2 => "Optical",
            1 => "Coaxial",
            _ => "Unknown",
        }
    }
}

/// Errors of this transport beyond plain I/O.
#[derive(Debug, thiserror::Error)]
pub enum HwdepError {
    #[error("no {HWDEP_NAME} hwdep device: is snd-fireface-ucx loaded with a UCX attached?")]
    NotFound,
    #[error("refused to write register {0:#06x}: it stores to the unit's flash")]
    Forbidden(u16),
    #[error("refused to write register {0:#06x}: it is the driver's, not the unit's")]
    DriverRegister(u16),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// The hwdep node of the `HWDEP_NAME` device listed in `proc_hwdep` (the
/// text of `/proc/asound/hwdep`, lines `CC-DD: name`).
pub fn node_in(proc_hwdep: &str, dev_dir: &Path) -> Option<PathBuf> {
    proc_hwdep.lines().find_map(|line| {
        let (ids, name) = line.split_once(':')?;
        if name.trim() != HWDEP_NAME {
            return None;
        }
        let (card, dev) = ids.trim().split_once('-')?;
        let card: u32 = card.parse().ok()?;
        let dev: u32 = dev.parse().ok()?;
        Some(dev_dir.join(format!("hwC{card}D{dev}")))
    })
}

/// A Fireface UCX's DSP, through `snd-fireface-ucx`'s hwdep node.
pub struct UcxHwdep<I: HwdepIo = FileIo> {
    io: I,
}

impl UcxHwdep<FileIo> {
    /// The first UCX on the system.
    pub fn find() -> Result<Self, HwdepError> {
        let proc = std::fs::read_to_string("/proc/asound/hwdep").unwrap_or_default();
        let node = node_in(&proc, Path::new("/dev/snd")).ok_or(HwdepError::NotFound)?;
        Self::open(&node)
    }

    /// The UCX behind hwdep node `path`.
    pub fn open(path: &Path) -> Result<Self, HwdepError> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        Ok(Self::with_io(FileIo(file)))
    }
}

impl<I: HwdepIo> UcxHwdep<I> {
    pub fn with_io(io: I) -> Self {
        Self { io }
    }

    pub fn status(&mut self) -> Result<Status, HwdepError> {
        let mut buf = [0u8; STATUS_STRUCT_BYTES];
        self.io.ioctl_read(IOCTL_STATUS, &mut buf)?;
        let w = |i: usize| u32::from_ne_bytes(buf[i * 4..i * 4 + 4].try_into().unwrap());
        Ok(Status {
            status: w(0),
            version: w(1),
            serial: [w(2), w(3)],
        })
    }

    /// Every register the driver knows, by register (mixer gains carry
    /// bit 14, see `latter::mix_reg`). The unit reports no mixer gain:
    /// those present are the ones the host wrote since the driver loaded.
    pub fn registers(&mut self) -> Result<BTreeMap<u16, u16>, HwdepError> {
        let mut buf = vec![0u8; REGS_BYTES];
        let mut len = 0;
        // One read normally returns the whole table; later reads of the
        // same file continue it, then return 0.
        while len < buf.len() {
            let n = self.io.read(&mut buf[len..])?;
            if n == 0 {
                break;
            }
            len += n;
        }
        Ok(buf[..len - len % 4]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| {
                let w = u32::from_ne_bytes(*c);
                (((w >> 16) & 0x7fff) as u16, w as u16)
            })
            .collect())
    }

    /// One reading of every meter bank the driver received.
    pub fn meters(&mut self) -> Result<Vec<meters::Packet>, HwdepError> {
        let mut buf = vec![0u8; METERS_STRUCT_BYTES];
        self.io.ioctl_read(IOCTL_METERS, &mut buf)?;
        Ok(buf
            .as_chunks::<{ 4 + METER_BYTES }>()
            .0
            .iter()
            .filter_map(|bank| {
                let len = u32::from_ne_bytes(bank[..4].try_into().unwrap()) as usize;
                meters::decode(&bank[4..4 + len.min(METER_BYTES)])
            })
            .collect())
    }

    /// Writes `(register, value)` pairs, in order, in calls of at most
    /// [`MAX_WORDS`]. The driver paces them to the unit. Nothing is sent
    /// if any word is refused: the flash registers (which the driver also
    /// refuses), and the driver's own pseudo-registers.
    pub fn write(&mut self, words: &[(u16, u16)]) -> Result<(), HwdepError> {
        for &(reg, value) in words {
            if latter::is_forbidden_word(latter::word(reg, value)) {
                return Err(HwdepError::Forbidden(reg));
            }
            if is_driver_register(reg) {
                return Err(HwdepError::DriverRegister(reg));
            }
        }
        for chunk in words.chunks(MAX_WORDS) {
            let bytes: Vec<u8> = chunk
                .iter()
                .flat_map(|&(reg, value)| ((reg as u32) << 16 | value as u32).to_ne_bytes())
                .collect();
            let n = self.io.write(&bytes)?;
            if n != bytes.len() {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    format!("short hwdep write: {n} of {} bytes", bytes.len()),
                )
                .into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for the driver: a register table, canned meters and
    /// status, and a record of every write call.
    #[derive(Default)]
    struct FakeDriver {
        table: Vec<u32>,
        read_pos: usize,
        writes: Vec<Vec<u32>>,
        meters: Vec<Vec<u8>>,
    }

    impl HwdepIo for FakeDriver {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            assert_eq!(buf.len() % 4, 0);
            assert!(
                buf.len() / 4 <= MAX_WORDS,
                "the driver takes 63 words a call"
            );
            self.writes.push(
                buf.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| u32::from_ne_bytes(*c))
                    .collect(),
            );
            Ok(buf.len())
        }

        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            // Hand the table out in pieces, as later reads may.
            let words = (buf.len() / 4).min(3).min(self.table.len() - self.read_pos);
            for i in 0..words {
                buf[i * 4..i * 4 + 4].copy_from_slice(&self.table[self.read_pos + i].to_ne_bytes());
            }
            self.read_pos += words;
            Ok(words * 4)
        }

        fn ioctl_read(&mut self, request: u64, buf: &mut [u8]) -> io::Result<()> {
            // The size field of the request must be the buffer's.
            assert_eq!((request >> 16 & 0x3fff) as usize, buf.len());
            match request {
                IOCTL_STATUS => {
                    let words = [0x7200_0e00u32, 0x0301_3100, 1234, 5678];
                    for (i, w) in words.iter().enumerate() {
                        buf[i * 4..i * 4 + 4].copy_from_slice(&w.to_ne_bytes());
                    }
                }
                IOCTL_METERS => {
                    for (b, data) in self.meters.iter().enumerate() {
                        let at = b * (4 + METER_BYTES);
                        buf[at..at + 4].copy_from_slice(&(data.len() as u32).to_ne_bytes());
                        buf[at + 4..at + 4 + data.len()].copy_from_slice(data);
                    }
                }
                _ => return Err(io::Error::from_raw_os_error(libc::ENOTTY)),
            }
            Ok(())
        }
    }

    /// The numbers `_IOR('U', 1, struct ucx_hwdep_meters)` and
    /// `_IOR('U', 2, struct ucx_hwdep_status)` give on Linux.
    #[test]
    fn ioctl_numbers_match_the_driver_header() {
        assert_eq!(METERS_STRUCT_BYTES, 2580);
        assert_eq!(IOCTL_METERS, 0x8a14_5501);
        assert_eq!(IOCTL_STATUS, 0x8010_5502);
    }

    #[test]
    fn the_node_is_found_by_name() {
        let proc = "00-00: HDA Codec 0\n02-00: Fireface UCX DSP\n";
        assert_eq!(
            node_in(proc, Path::new("/dev/snd")),
            Some(PathBuf::from("/dev/snd/hwC2D0"))
        );
        assert_eq!(node_in("00-00: HDA Codec 0\n", Path::new("/dev/snd")), None);
    }

    #[test]
    fn the_register_table_reads_across_short_reads() {
        let fake = FakeDriver {
            table: vec![0x3f99_0812, 0x1208_0001, 0x4002_9000, 0x0008_0014],
            ..Default::default()
        };
        let regs = UcxHwdep::with_io(fake).registers().unwrap();
        assert_eq!(regs.len(), 4);
        assert_eq!(regs[&0x4002], 0x9000);
        assert_eq!(regs[&0x1208], 1);
        assert_eq!(latter::mix_gain(regs[&0x4002]), latter::MIX_UNITY);
    }

    #[test]
    fn status_decodes() {
        let s = UcxHwdep::with_io(FakeDriver::default()).status().unwrap();
        assert_eq!(s.version, 0x0301_3100);
        assert_eq!(s.serial, [1234, 5678]);
        assert_eq!(s.clock(), "Internal");
        assert_eq!(s.rate_code(), Some(2));
    }

    #[test]
    fn meters_decode_every_bank_received() {
        let mut packet = vec![0u8; meters::PACKET_LEN];
        packet[0xa0..0xa4].copy_from_slice(&(1u32 << 27).to_le_bytes());
        packet[0xf0..0xf4].copy_from_slice(&0x1111_1111u32.to_le_bytes());
        // One full bank, one empty (not received this time).
        let fake = FakeDriver {
            meters: vec![packet, Vec::new()],
            ..Default::default()
        };
        let banks = UcxHwdep::with_io(fake).meters().unwrap();
        assert_eq!(banks.len(), 1);
        assert_eq!(banks[0].bank, meters::Bank::InputsPost);
        assert!(banks[0].peak_db[0].unwrap().abs() < 0.01);
    }

    #[test]
    fn writes_go_in_calls_of_at_most_63_words() {
        let words: Vec<(u16, u16)> = (0..100).map(|i| (latter::mix_reg(0, i), 0x9000)).collect();
        let mut dev = UcxHwdep::with_io(FakeDriver::default());
        dev.write(&words).unwrap();
        let calls = &dev.io.writes;
        assert_eq!(calls.iter().map(Vec::len).collect::<Vec<_>>(), [63, 37]);
        assert_eq!(calls[0][2], 0x4002_9000, "no parity: the driver adds it");
    }

    #[test]
    fn flash_and_driver_registers_are_refused_before_anything_is_sent() {
        let mut dev = UcxHwdep::with_io(FakeDriver::default());
        assert!(matches!(
            dev.write(&[(0x0008, 1), (latter::REG_FLASH_STORE, 0x0910)]),
            Err(HwdepError::Forbidden(0x3e06))
        ));
        assert!(matches!(
            dev.write(&[(REG_LOOPBACK + 3, 1)]),
            Err(HwdepError::DriverRegister(_))
        ));
        assert!(
            dev.io.writes.is_empty(),
            "the good word before it was not sent either"
        );
    }
}
