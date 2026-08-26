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
        let card_list = read_card_list();

        for (card_idx, _name) in &card_list {
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
                info!("Detected target device: {} ({})", name, card_name);
                let mixer = Mixer::new(&card_name, false)?;
                let _ = mixer.find_selem(&SelemId::new("", 0));
                return Ok(Self { mixer, card_name });
            }
        }

        Err(Error::DeviceNotFound {
            model: card_substring.to_string(),
        })
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

    /// Handle pending ALSA events.
    pub fn handle_events(&self) -> Result<u32, Error> {
        Ok(0)
    }

    /// Name of the ALSA card (e.g. "hw:0").
    pub fn card_name(&self) -> &str {
        &self.card_name
    }
}
