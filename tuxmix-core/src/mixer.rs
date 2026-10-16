use std::fs;

use alsa::mixer::{Mixer, Selem, SelemId};
use alsa::Ctl;
use log::{debug, info};

use crate::error::Error;

/// Wraps an ALSA mixer handle and provides device discovery helpers.
pub struct AlsaMixer {
    mixer: Mixer,
    card_name: String,
}

/// Parse card index and name from `/proc/asound/cards`.
fn read_card_list() -> Vec<(i32, String)> {
    let mut cards = Vec::new();

    let content = match fs::read_to_string("/proc/asound/cards") {
        Ok(c) => c,
        Err(_) => return cards,
    };

    for line in content.lines() {
        // Lines look like: " 0 [HDMI           ]: HDA-Intel ..."
        let trimmed = line.trim();
        let idx: i32 = match trimmed.split(' ').next().unwrap_or("").parse() {
            Ok(n) => n,
            Err(_) => continue,
        };

        let name = if let Some(start) = trimmed.find('[') {
            if let Some(end) = trimmed.find(']') {
                trimmed[start + 1..end].trim().to_string()
            } else {
                continue;
            }
        } else {
            continue;
        };

        cards.push((idx, name));
    }

    cards
}

impl AlsaMixer {
    /// Scan all ALSA cards and open the mixer for the first one whose
    /// name (from the card info) contains the given `card_substring`.
    ///
    /// Reads `/proc/asound/cards` first to avoid spurious ALSA library
    /// error messages when probing non-existent card indices.
    pub fn open_by_card_name(card_substring: &str) -> Result<Self, Error> {
        match Self::resolve_card(card_substring) {
            Some((card_name, name)) => {
                info!("Detected target device: {} ({})", name, card_name);
                let mixer = Mixer::new(&card_name, false)?;
                let _ = mixer.find_selem(&SelemId::new("", 0));
                Ok(Self { mixer, card_name })
            }
            None => Err(Error::DeviceNotFound {
                model: card_substring.to_string(),
            }),
        }
    }

    /// The scan half of `open_by_card_name`, opening nothing: the
    /// `(card_name, card_info_name)` of the first card whose name
    /// contains `card_substring`. Kept quiet on purpose: this is what the
    /// once-a-second liveness probe (see `RmeDevice::is_connected`) runs,
    /// and the `info!` belongs to the open, not to the scan.
    pub(crate) fn resolve_card(card_substring: &str) -> Option<(String, String)> {
        for (card_idx, _name) in &read_card_list() {
            let card_name = format!("hw:{}", card_idx);

            let ctl = match Ctl::new(&card_name, false) {
                Err(_) => continue,
                Ok(c) => c,
            };

            let info = match ctl.card_info() {
                Err(_) => continue,
                Ok(i) => i,
            };

            let name = info.get_name().unwrap_or_default().to_string();
            debug!("Found ALSA card: {} — {}", card_name, name);

            if name.contains(card_substring) {
                return Some((card_name, name));
            }
        }

        None
    }

    /// The kernel's enumeration number (`devnum`) of the USB device behind
    /// an ALSA card (`hw:N`). It increments on every re-enumeration, while
    /// a re-created card reuses the very same ALSA index and names: this
    /// is the one thing that tells a replug or a personality switch apart
    /// from the card a mixer handle was opened on. `None` when the card is
    /// gone, is not USB, or its sysfs entry cannot be read.
    pub(crate) fn card_devnum(card_name: &str) -> Option<String> {
        let idx = card_name.strip_prefix("hw:")?;
        let device = std::fs::canonicalize(format!("/sys/class/sound/card{idx}/device")).ok()?;
        // `.../usb1/1-2/1-2:1.0` — the USB device is the interface's
        // parent, and `devnum` lives there.
        let usb_device = device.parent()?;
        let devnum = std::fs::read_to_string(usb_device.join("devnum")).ok()?;
        Some(devnum.trim().to_string())
    }

    /// Find a mixer element by name.
    pub fn find_selem(&self, name: &str, index: u32) -> Option<Selem<'_>> {
        let id = SelemId::new(name, index);
        self.mixer.find_selem(&id)
    }

    /// Iterate over all simple mixer elements, yielding (name, Selem) pairs.
    pub fn iter_selems(&self) -> Vec<(String, Selem<'_>)> {
        let mut elems = Vec::new();
        for elem in self.mixer.iter() {
            if let Some(selem) = Selem::new(elem) {
                let id = selem.get_id();
                if let Ok(name) = id.get_name() {
                    elems.push((name.to_string(), selem));
                }
            }
        }
        elems
    }

    /// Handle pending ALSA events: returns how many were processed.
    ///
    /// alsa-lib keeps the simple-mixer values it read at load time and
    /// only refreshes them from the events it is given here, so until
    /// this runs every `get_*` returns the value the control had when
    /// the mixer was opened, whatever the driver or another client did
    /// since.  Does not block when nothing is pending.
    pub fn handle_events(&self) -> Result<u32, Error> {
        Ok(self.mixer.handle_events()?)
    }

    /// Name of the ALSA card (e.g. "hw:0") — also lets a caller open a
    /// *different* interface on the exact same physical card (e.g. the
    /// capture PCM, for real-time level metering) without re-scanning
    /// `/proc/asound/cards` itself.
    pub fn card_name(&self) -> &str {
        &self.card_name
    }
}
