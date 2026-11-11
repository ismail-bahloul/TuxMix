// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Ismaïl Bahloul

//! RME units on the USB bus that TuxMix cannot drive yet.
//!
//! When no supported device opens, the no-device screen used to say only
//! "check that the interface is plugged in", even with a Fireface UCX
//! sitting on the bus. This module looks at the bus itself
//! (`/sys/bus/usb/devices`, no libusb and no permissions needed) and names
//! what it finds, so the owner learns that the unit was seen and is not
//! supported, instead of hunting for a cabling problem.
//!
//! Only the IDs we have a public source for are named here. Any other unit
//! under RME's vendor ID is reported with the product string the device
//! itself gives.

use std::path::Path;

/// RME's USB vendor ID.
pub const RME_VENDOR: u16 = 0x2a39;

/// The Babyface Pro in its proprietary mode: the one TuxMix drives.
pub const BABYFACE_PRO: (u16, u16) = (RME_VENDOR, 0x3fc0);

/// Known units, by (vendor, product). The Fireface UC enumerates under
/// SMSC's vendor ID in its PC mode, so it is listed by full ID rather than
/// matched on RME's vendor alone.
const KNOWN: &[((u16, u16), &str)] = &[
    ((RME_VENDOR, 0x3fb0), "Babyface Pro in class-compliant mode"),
    ((0x0424, 0x3fc5), "Fireface UC (PC mode)"),
    ((RME_VENDOR, 0x3fc6), "Fireface UC (Mac mode)"),
    ((RME_VENDOR, 0x3fc9), "Fireface UCX"),
];

/// One USB device as sysfs describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbDevice {
    pub vendor: u16,
    pub product: u16,
    /// The device's own product string, without the serial number some
    /// RME units append in parentheses.
    pub name: String,
}

/// Every USB device under `root` (normally `/sys/bus/usb/devices`).
/// Entries without an `idVendor` (interfaces, hubs' ports) are skipped.
pub fn scan(root: &Path) -> Vec<UsbDevice> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let read = |dir: &Path, f: &str| {
        std::fs::read_to_string(dir.join(f))
            .ok()
            .map(|s| s.trim().to_string())
    };
    let hex = |s: Option<String>| s.and_then(|s| u16::from_str_radix(&s, 16).ok());
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        let (Some(vendor), Some(product)) =
            (hex(read(&dir, "idVendor")), hex(read(&dir, "idProduct")))
        else {
            continue;
        };
        let name = read(&dir, "product").unwrap_or_default();
        let name = match name.find(" (") {
            Some(i) if name.ends_with(')') => name[..i].to_string(),
            _ => name,
        };
        found.push(UsbDevice {
            vendor,
            product,
            name,
        });
    }
    found.sort_by_key(|d| (d.vendor, d.product));
    found
}

/// What to call `dev` if it is an RME unit TuxMix does not drive.
/// `None` for the supported Babyface Pro and for anything not RME.
pub fn unsupported_name(dev: &UsbDevice) -> Option<String> {
    let id = (dev.vendor, dev.product);
    if id == BABYFACE_PRO {
        return None;
    }
    if let Some((_, name)) = KNOWN.iter().find(|(k, _)| *k == id) {
        return Some((*name).to_string());
    }
    if dev.vendor != RME_VENDOR {
        return None;
    }
    Some(if dev.name.is_empty() {
        "An RME unit".to_string()
    } else {
        format!("RME {}", dev.name.trim_start_matches("RME "))
    })
}

/// The line the no-device screen shows for the first unsupported RME unit
/// on the bus, or `None` when there is none.
pub fn unsupported_hint_in(root: &Path) -> Option<String> {
    let dev = scan(root).into_iter().find_map(|d| {
        let name = unsupported_name(&d)?;
        Some((name, d.vendor, d.product))
    })?;
    let (name, vendor, product) = dev;
    if (vendor, product) == (RME_VENDOR, 0x3fb0) {
        return Some(format!(
            "{name} found (USB {vendor:04x}:{product:04x}). TuxMix needs it in its normal \
             (proprietary) mode: hold SELECT and DIM while plugging in the USB cable to switch."
        ));
    }
    Some(format!(
        "{name} found (USB {vendor:04x}:{product:04x}). TuxMix does not support this unit \
         yet; it drives the Babyface Pro FS today. If you own one and can help, see \
         CONTRIBUTING.md."
    ))
}

/// [`unsupported_hint_in`] on the running system's bus.
pub fn unsupported_hint() -> Option<String> {
    unsupported_hint_in(Path::new("/sys/bus/usb/devices"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway sysfs tree: one directory per device, with the files
    /// the scan reads.
    fn fake_bus(name: &str, devices: &[(&str, &str, &str, &str)]) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("tuxmix-usb-ids-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (dir, vendor, product, label) in devices {
            let d = root.join(dir);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("idVendor"), format!("{vendor}\n")).unwrap();
            std::fs::write(d.join("idProduct"), format!("{product}\n")).unwrap();
            std::fs::write(d.join("product"), format!("{label}\n")).unwrap();
        }
        // An interface entry: no idVendor, must be skipped.
        std::fs::create_dir_all(root.join("1-2:1.0")).unwrap();
        root
    }

    #[test]
    fn a_ucx_is_named_and_the_babyface_is_not() {
        let root = fake_bus(
            "ucx",
            &[
                ("1-2", "2a39", "3fc0", "Babyface Pro (73055480)"),
                ("1-3", "2a39", "3fc9", "Fireface UCX (23456789)"),
                ("1-4", "046d", "c52b", "USB Receiver"),
            ],
        );
        let hint = unsupported_hint_in(&root).unwrap();
        assert!(
            hint.starts_with("Fireface UCX found (USB 2a39:3fc9)"),
            "{hint}"
        );
        assert!(!hint.contains("23456789"), "the serial stays out: {hint}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn only_the_supported_babyface_gives_no_hint() {
        let root = fake_bus("bf", &[("1-2", "2a39", "3fc0", "Babyface Pro (73055480)")]);
        assert_eq!(unsupported_hint_in(&root), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_uc_in_pc_mode_is_found_under_smsc_but_other_smsc_devices_are_not() {
        let root = fake_bus(
            "uc",
            &[
                ("1-5", "0424", "2514", "USB 2.0 Hub"),
                ("1-6", "0424", "3fc5", "Fireface UC Win"),
            ],
        );
        let hint = unsupported_hint_in(&root).unwrap();
        assert!(
            hint.starts_with("Fireface UC (PC mode) found (USB 0424:3fc5)"),
            "{hint}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_unlisted_rme_unit_uses_its_own_product_string() {
        let root = fake_bus("other", &[("2-1", "2a39", "0001", "Test Unit (11112222)")]);
        let hint = unsupported_hint_in(&root).unwrap();
        assert!(
            hint.starts_with("RME Test Unit found (USB 2a39:0001)"),
            "{hint}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn class_compliant_babyface_says_how_to_switch() {
        let root = fake_bus("cc", &[("1-2", "2a39", "3fb0", "Babyface Pro (73055480)")]);
        let hint = unsupported_hint_in(&root).unwrap();
        assert!(hint.contains("proprietary"), "{hint}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_missing_bus_is_no_hint() {
        assert_eq!(
            unsupported_hint_in(Path::new("/nonexistent/tuxmix/bus")),
            None
        );
    }
}
