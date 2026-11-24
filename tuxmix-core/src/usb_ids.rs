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

use crate::profile::{Support, UsbMode};

/// RME's USB vendor ID.
pub const RME_VENDOR: u16 = 0x2a39;

/// The Babyface Pro in its proprietary mode: the one TuxMix drives.
pub const BABYFACE_PRO: (u16, u16) = (RME_VENDOR, 0x3fc0);

/// RME units with no TuxMix profile, by vendor and product, and where
/// their owner can look instead. The units with a profile are named from
/// it (see [`crate::profiles::by_usb`]). The Fireface UC and the Babyface
/// classic enumerate under SMSC's vendor ID in some modes, so they are
/// listed by full ID rather than matched on RME's vendor alone.
///
/// Sources: TotalControl's protocol specifications (UC, Babyface classic),
/// and mainline `snd-usb-audio` (`sound/usb/quirks.c`,
/// `sound/usb/mixer_quirks.c`) for the Digiface USB and the ADI-2.
const KNOWN: &[(u16, u16, &str, Option<&str>)] = &[
    (0x0424, 0x3fc5, "Fireface UC (PC mode)", Some(TOTALCONTROL)),
    (0x2a39, 0x3fc5, "Fireface UC (PC mode)", Some(TOTALCONTROL)),
    (0x2a39, 0x3fce, "Fireface UC (PC mode)", Some(TOTALCONTROL)),
    (0x0424, 0x3fc6, "Fireface UC (Mac mode)", Some(TOTALCONTROL)),
    (0x2a39, 0x3fc6, "Fireface UC (Mac mode)", Some(TOTALCONTROL)),
    (0x0424, 0x3fc7, "Babyface", Some(TOTALCONTROL)),
    (0x2a39, 0x3fc7, "Babyface", Some(TOTALCONTROL)),
    (0x2a39, 0x3f8c, "Digiface USB", Some(SND_USB_AUDIO)),
    (0x2a39, 0x3fa0, "Digiface USB", Some(SND_USB_AUDIO)),
    (0x2a39, 0x3fd2, "ADI-2 Pro", Some(SND_USB_AUDIO)),
    (0x2a39, 0x3fd3, "ADI-2 DAC", Some(SND_USB_AUDIO)),
];

const TOTALCONTROL: &str = "TotalControl (https://github.com/jspx-projects/TotalControl) drives it";
const SND_USB_AUDIO: &str = "the kernel's snd-usb-audio drives it";
const REALBOGART_UCX: &str =
    "snd-fireface-ucx (https://github.com/realbogart/fireface-ucx-linux) and TotalControl \
     (https://github.com/jspx-projects/TotalControl) drive it";
const OSCMIX: &str = "oscmix (https://github.com/michaelforney/oscmix) controls its mixer in \
     class-compliant mode";

/// Where the owner of a profiled but undriven unit can look, by profile id.
fn elsewhere_for_profile(id: &str) -> Option<&'static str> {
    match id {
        "ucx" => Some(REALBOGART_UCX),
        "ucx2" | "ff802" | "ufx2" | "ufx3" | "ufxplus" => Some(OSCMIX),
        _ => None,
    }
}

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

/// What TuxMix knows about an RME unit it does not drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported {
    /// The unit's name, with its mode when that matters.
    pub name: String,
    /// The profile id, when the simulation can show the unit's layout.
    pub profile: Option<&'static str>,
    /// A project or driver that does drive it.
    pub elsewhere: Option<&'static str>,
}

/// What TuxMix knows about `dev` if it is an RME unit TuxMix does not
/// drive. `None` for the supported Babyface Pro and for anything not RME.
pub fn unsupported(dev: &UsbDevice) -> Option<Unsupported> {
    let id = (dev.vendor, dev.product);
    if id == BABYFACE_PRO {
        return None;
    }
    if let Some(p) = crate::profiles::by_usb(dev.vendor, dev.product) {
        let cc = p.usb_mode(dev.vendor, dev.product) == Some(UsbMode::ClassCompliant);
        return Some(Unsupported {
            name: if cc {
                format!("{} in class-compliant mode", p.model_name)
            } else {
                p.model_name.to_string()
            },
            profile: (p.support == Support::SimulationOnly).then_some(p.id),
            elsewhere: elsewhere_for_profile(p.id),
        });
    }
    if let Some((_, _, name, elsewhere)) = KNOWN.iter().find(|(v, p, _, _)| (*v, *p) == id) {
        return Some(Unsupported {
            name: (*name).to_string(),
            profile: None,
            elsewhere: *elsewhere,
        });
    }
    if dev.vendor != RME_VENDOR {
        return None;
    }
    Some(Unsupported {
        name: if dev.name.is_empty() {
            "An RME unit".to_string()
        } else {
            format!("RME {}", dev.name.trim_start_matches("RME "))
        },
        profile: None,
        elsewhere: None,
    })
}

/// What to call `dev` if it is an RME unit TuxMix does not drive.
/// `None` for the supported Babyface Pro and for anything not RME.
pub fn unsupported_name(dev: &UsbDevice) -> Option<String> {
    unsupported(dev).map(|u| u.name)
}

/// The line the no-device screen shows for the first unsupported RME unit
/// on the bus, or `None` when there is none.
pub fn unsupported_hint_in(root: &Path) -> Option<String> {
    let (u, vendor, product) = scan(root)
        .into_iter()
        .find_map(|d| Some((unsupported(&d)?, d.vendor, d.product)))?;
    let name = &u.name;
    if (vendor, product) == (RME_VENDOR, 0x3fb0) {
        return Some(format!(
            "{name} found (USB {vendor:04x}:{product:04x}). TuxMix needs it in its normal \
             (proprietary) mode: hold SELECT and DIM while plugging in the USB cable to switch."
        ));
    }
    let mut hint = format!(
        "{name} found (USB {vendor:04x}:{product:04x}). TuxMix does not support this unit \
         yet; it drives the Babyface Pro FS today."
    );
    if let Some(elsewhere) = u.elsewhere {
        hint.push_str(&format!(" Meanwhile, {elsewhere}."));
    }
    if let Some(id) = u.profile {
        hint.push_str(&format!(
            " `TUXMIX_MOCK_MODEL={id} tuxmix --mock` shows its layout."
        ));
    }
    hint.push_str(" If you own one and can help, see CONTRIBUTING.md.");
    Some(hint)
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
        assert!(hint.contains("TotalControl"), "{hint}");
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
    fn a_profiled_unit_points_to_the_simulation_and_elsewhere() {
        let root = fake_bus(
            "ucx2",
            &[("3-1", "2a39", "3f82", "Fireface UCX II (24000000)")],
        );
        let hint = unsupported_hint_in(&root).unwrap();
        assert!(
            hint.starts_with("Fireface UCX II found (USB 2a39:3f82)"),
            "{hint}"
        );
        assert!(hint.contains("TUXMIX_MOCK_MODEL=ucx2"), "{hint}");
        assert!(hint.contains("oscmix"), "{hint}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_class_compliant_ucx_says_so() {
        let dev = UsbDevice {
            vendor: RME_VENDOR,
            product: 0x3fb9,
            name: "Fireface UCX".into(),
        };
        assert_eq!(
            unsupported_name(&dev).unwrap(),
            "Fireface UCX in class-compliant mode"
        );
    }

    #[test]
    fn units_the_kernel_drives_say_so() {
        let dev = UsbDevice {
            vendor: RME_VENDOR,
            product: 0x3f8c,
            name: String::new(),
        };
        let u = unsupported(&dev).unwrap();
        assert_eq!(u.name, "Digiface USB");
        assert!(u.elsewhere.unwrap().contains("snd-usb-audio"));
    }

    #[test]
    fn a_missing_bus_is_no_hint() {
        assert_eq!(
            unsupported_hint_in(Path::new("/nonexistent/tuxmix/bus")),
            None
        );
    }
}
