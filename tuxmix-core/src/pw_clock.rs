//! The PipeWire graph clock: sample rate and buffer size (the "quantum").
//!
//! With the kernel driver, the Babyface's rate and buffer are not the
//! mixer's to set: the sound server opens the card, so whatever PipeWire's
//! graph runs at is what the hardware runs at. These are the settings a
//! Windows user finds in RME's driver dialog (sample rate, ASIO buffer
//! size); on Linux they live in PipeWire's `settings` metadata, which
//! `pw-metadata` reads and writes at runtime:
//!
//! * `clock.rate` / `clock.quantum`: the defaults the graph uses;
//! * `clock.force-rate` / `clock.force-quantum`: overrides, `0` = none.
//!
//! The overrides are system-wide (every PipeWire client follows them) and
//! last until PipeWire restarts. `pw-metadata` ships with PipeWire, so this
//! needs no build dependency, like `pw_meter`'s use of `pw-record`.

use std::process::Command;

/// The graph clock as PipeWire reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PwClock {
    pub rate: u32,
    pub quantum: u32,
    /// `0` = not forced.
    pub force_rate: u32,
    /// `0` = not forced.
    pub force_quantum: u32,
}

impl PwClock {
    /// What the graph runs at: the forced value if any, else the default.
    pub fn effective_rate(&self) -> u32 {
        if self.force_rate > 0 {
            self.force_rate
        } else {
            self.rate
        }
    }

    pub fn effective_quantum(&self) -> u32 {
        if self.force_quantum > 0 {
            self.force_quantum
        } else {
            self.quantum
        }
    }

    /// One buffer's worth of time, in milliseconds.
    pub fn latency_ms(&self) -> f32 {
        let rate = self.effective_rate();
        if rate == 0 {
            return 0.0;
        }
        self.effective_quantum() as f32 * 1000.0 / rate as f32
    }
}

/// Parses `pw-metadata -n settings 0` output, lines like
/// `update: id:0 key:'clock.rate' value:'48000' type:''`. `None` when the
/// two defaults are missing (no PipeWire, or not its settings object).
pub fn parse(output: &str) -> Option<PwClock> {
    let value_of = |key: &str| -> Option<u32> {
        let needle = format!("key:'{key}' value:'");
        let line = output.lines().find(|l| l.contains(&needle))?;
        let rest = &line[line.find(&needle)? + needle.len()..];
        rest[..rest.find('\'')?].trim().parse().ok()
    };
    Some(PwClock {
        rate: value_of("clock.rate")?,
        quantum: value_of("clock.quantum")?,
        force_rate: value_of("clock.force-rate").unwrap_or(0),
        force_quantum: value_of("clock.force-quantum").unwrap_or(0),
    })
}

/// Reads the live clock. `None` if PipeWire (or `pw-metadata`) is absent.
pub fn read() -> Option<PwClock> {
    let out = Command::new("pw-metadata")
        .args(["-n", "settings", "0"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse(&String::from_utf8_lossy(&out.stdout))
}

/// Forces the graph rate; `0` lifts the override.
pub fn force_rate(rate: u32) -> Result<(), String> {
    set("clock.force-rate", rate)
}

/// Forces the graph quantum (buffer size in samples); `0` lifts it.
pub fn force_quantum(quantum: u32) -> Result<(), String> {
    set("clock.force-quantum", quantum)
}

fn set(key: &str, value: u32) -> Result<(), String> {
    let status = Command::new("pw-metadata")
        .args(["-n", "settings", "0", key, &value.to_string()])
        .output()
        .map_err(|e| format!("pw-metadata: {e}"))?
        .status;
    if status.success() {
        Ok(())
    } else {
        Err(format!("pw-metadata {key} {value} failed ({status})"))
    }
}

/// What the card itself is running at, from the kernel's view of its open
/// stream (`/proc/asound/<card>/pcm0p/sub0/hw_params`, else the capture
/// side). This is the truth the graph clock cannot give: the sound server
/// may open the card with its own fixed period (WirePlumber's
/// `api.alsa.period-size`, IRQ mode), and then the card's latency does not
/// follow the quantum at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardStream {
    pub rate: u32,
    /// Frames per period: one hardware interrupt's worth.
    pub period: u32,
    /// Frames in the whole ring buffer.
    pub buffer: u32,
}

impl CardStream {
    /// One period's worth of time, in milliseconds.
    pub fn period_ms(&self) -> f32 {
        if self.rate == 0 {
            return 0.0;
        }
        self.period as f32 * 1000.0 / self.rate as f32
    }
}

/// The smallest period the kernel driver accepts, from its module
/// parameter (`urb_frames_min`, 32 by default; the driver's period floor
/// is one URB). `None` when the module is not loaded.
pub fn driver_period_floor() -> Option<u32> {
    std::fs::read_to_string("/sys/module/snd_usb_babyface_pro/parameters/urb_frames_min")
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Parses a `hw_params` file. `None` for "closed" (the card is idle).
pub fn parse_hw_params(text: &str) -> Option<CardStream> {
    let field = |name: &str| -> Option<u32> {
        let line = text.lines().find(|l| l.starts_with(name))?;
        line[name.len()..]
            .trim_start_matches(':')
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    };
    Some(CardStream {
        rate: field("rate")?,
        period: field("period_size")?,
        buffer: field("buffer_size")?,
    })
}

/// The open *playback* stream of the ALSA card whose `id` is `card_id`.
/// `None` when no such card exists or nothing is playing. Playback only: the
/// capture side is held open by TuxMix's own input meter
/// (`capture_meter`), so reading it would report the app's own stream
/// (it once showed the meter's period as the card's).
pub fn card_stream(card_id: &str) -> Option<CardStream> {
    let cards = std::fs::read_dir("/proc/asound").ok()?;
    for entry in cards.flatten() {
        let dir = entry.path();
        let is_card = dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("card"));
        let id = std::fs::read_to_string(dir.join("id")).unwrap_or_default();
        if !is_card || id.trim() != card_id {
            continue;
        }
        let text = std::fs::read_to_string(dir.join("pcm0p/sub0/hw_params")).ok()?;
        return parse_hw_params(&text);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Found \"settings\" metadata 31
update: id:0 key:'log.level' value:'2' type:''
update: id:0 key:'clock.rate' value:'48000' type:''
update: id:0 key:'clock.allowed-rates' value:'[ 48000 ]' type:''
update: id:0 key:'clock.quantum' value:'64' type:''
update: id:0 key:'clock.min-quantum' value:'32' type:''
update: id:0 key:'clock.max-quantum' value:'2048' type:''
update: id:0 key:'clock.force-quantum' value:'0' type:''
update: id:0 key:'clock.force-rate' value:'0' type:''
";

    #[test]
    fn parses_real_pw_metadata_output() {
        let c = parse(SAMPLE).unwrap();
        assert_eq!(
            c,
            PwClock {
                rate: 48_000,
                quantum: 64,
                force_rate: 0,
                force_quantum: 0
            }
        );
        // `clock.quantum` must not be confused with `clock.min-quantum`.
        assert_eq!(c.effective_quantum(), 64);
        assert!((c.latency_ms() - 1.333).abs() < 0.01);
    }

    #[test]
    fn forced_values_win() {
        let c = parse(
            &SAMPLE
                .replace(
                    "clock.force-quantum' value:'0'",
                    "clock.force-quantum' value:'256'",
                )
                .replace(
                    "clock.force-rate' value:'0'",
                    "clock.force-rate' value:'96000'",
                ),
        )
        .unwrap();
        assert_eq!(c.effective_rate(), 96_000);
        assert_eq!(c.effective_quantum(), 256);
    }

    /// Against the live PipeWire: run by hand (`--ignored`). Only ever
    /// writes the values already in place (`0` = no override when none is
    /// set), so it changes nothing on the machine it runs on.
    #[test]
    #[ignore]
    fn live_read_and_neutral_write() {
        let before = read().expect("PipeWire is running");
        assert!(before.effective_rate() > 0 && before.effective_quantum() > 0);
        force_quantum(before.force_quantum).unwrap();
        force_rate(before.force_rate).unwrap();
        assert_eq!(read(), Some(before));
    }

    #[test]
    fn parses_a_running_cards_hw_params() {
        let text = "access: MMAP_INTERLEAVED\nformat: S32_LE\nsubformat: STD\n\
                    channels: 12\nrate: 48000 (48000/1)\nperiod_size: 256\n\
                    buffer_size: 512\n";
        let s = parse_hw_params(text).unwrap();
        assert_eq!(
            s,
            CardStream {
                rate: 48_000,
                period: 256,
                buffer: 512
            }
        );
        assert!((s.period_ms() - 5.333).abs() < 0.01);
        assert_eq!(parse_hw_params("closed\n"), None);
    }

    #[test]
    fn no_pipewire_settings_is_none() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("Found \"settings\" metadata 31\n"), None);
    }
}
