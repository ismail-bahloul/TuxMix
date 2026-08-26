//! ALSA device and mixer scanner.
//!
//! Lists all ALSA cards and their simple mixer elements with
//! current values. Essential for discovering the exact control
//! names exposed by your RME interface.
//!
//! Usage:
//!     cargo run --example alsa-scan
//!     cargo run --example alsa-scan Babyface   # filter cards by name

use alsa::mixer::{Selem, SelemChannelId};
use alsa::Ctl;
use std::env;
use std::fs;

fn list_cards() -> Vec<(i32, String)> {
    let mut cards = Vec::new();

    // Read the list of ALSA cards from /proc/asound/cards
    let content = match fs::read_to_string("/proc/asound/cards") {
        Ok(c) => c,
        Err(_) => {
            // Fallback: try enumerating via Ctl
            for i in 0..16 {
                let card_name = format!("hw:{}", i);
                if let Ok(ctl) = Ctl::new(&card_name, false) {
                    if let Ok(info) = ctl.card_info() {
                        let name = info.get_name().unwrap_or_default().to_string();
                        if !name.is_empty() {
                            cards.push((i, name));
                            continue;
                        }
                    }
                }
                break;
            }
            return cards;
        }
    };

    for line in content.lines() {
        // Lines look like: " 0 [HDMI           ]: HDA-Intel ..."
        let trimmed = line.trim();
        if let Ok(idx) = trimmed.split(' ').next().unwrap_or("").parse::<i32>() {
            // Extract the name between [ and ]
            if let Some(start) = trimmed.find('[') {
                if let Some(end) = trimmed.find(']') {
                    let name = trimmed[start + 1..end].trim().to_string();
                    cards.push((idx, name));
                } else {
                    cards.push((idx, trimmed.to_string()));
                }
            } else {
                cards.push((idx, trimmed.to_string()));
            }
        }
    }

    cards
}

fn main() {
    let filter = env::args().nth(1);

    println!("ALSA Card Scanner");
    println!(
        "  Filter: {}",
        filter.as_deref().unwrap_or("(none — showing all)")
    );
    println!();

    let cards = list_cards();
    if cards.is_empty() {
        eprintln!("No ALSA cards found.");
        return;
    }

    for (card_idx, desc) in &cards {
        // Apply text filter
        if let Some(ref f) = filter {
            if !desc.contains(f) {
                continue;
            }
        }

        let card_name = format!("hw:{}", card_idx);

        // Get long info via CTL
        let ctl = Ctl::new(&card_name, false).ok();
        let (longname, driver) = if let Some(ref c) = ctl {
            match c.card_info() {
                Ok(info) => (
                    info.get_longname().unwrap_or_default().to_string(),
                    info.get_driver().unwrap_or_default().to_string(),
                ),
                Err(_) => (String::new(), String::new()),
            }
        } else {
            (String::new(), String::new())
        };

        println!("--- Card #{}  {} ---", card_idx, desc);
        if !longname.is_empty() {
            println!("    longname: {}", longname);
        }
        if !driver.is_empty() {
            println!("    driver:   {}", driver);
        }
        println!();

        scan_mixer(&card_name);
        println!();
    }
}

fn scan_mixer(card: &str) {
    let mixer = match alsa::mixer::Mixer::new(card, false) {
        Err(e) => {
            println!("    no simple mixer interface: {}", e);
            return;
        }
        Ok(m) => m,
    };

    let mut count = 0;
    for elem in mixer.iter() {
        if let Some(selem) = Selem::new(elem) {
            let id = selem.get_id();
            if let Ok(ename) = id.get_name() {
                count += 1;
                print_element(&selem, ename);
            }
        }
    }

    if count == 0 {
        println!("    (no simple mixer elements)");
    } else {
        println!("    -- {} element(s) total --", count);
    }
}

fn print_element(selem: &Selem, name: &str) {
    let mono = SelemChannelId::mono();

    // Try to read as playback volume
    if selem.has_playback_volume() {
        let range = selem.get_playback_volume_range();
        if let Ok(vol) = selem.get_playback_volume(mono) {
            let pct = if range.1 > range.0 {
                (vol - range.0) as f64 / (range.1 - range.0) as f64 * 100.0
            } else {
                0.0
            };
            println!(
                "    VOL  {:<30}  value={:<6}  range=[{}-{}]  {:.0}%",
                name, vol, range.0, range.1, pct
            );
            return;
        }
    }

    // Try to read as playback switch
    if selem.has_playback_switch() {
        if let Ok(v) = selem.get_playback_switch(mono) {
            println!(
                "    SW   {:<30}  {}",
                name,
                if v != 0 { "ON" } else { "OFF" }
            );
            return;
        }
    }

    // Try to read as enum
    if let Ok(item) = selem.get_enum_item(mono) {
        if let Ok(count) = selem.get_enum_items() {
            if count > 0 {
                println!(
                    "    ENUM {:<30}  selected={}  ({} items)",
                    name, item, count
                );
            } else {
                println!("    ENUM {:<30}  selected={}", name, item);
            }
        } else {
            println!("    ENUM {:<30}  selected={}", name, item);
        }
        return;
    }

    // Fallback
    println!("    ???  {:<30}  (unhandled type)", name);
}
