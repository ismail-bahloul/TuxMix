//! Front-panel emulator — turns physical front-panel events into mixer
//! actions, mirroring what TotalMix does on Windows.
//!
//! The front panel is HOST-DRIVEN: pressing MIX/SELECT or turning the
//! wheel only changes the device's `0x17` status readback (byte 0/1
//! gain 0x80 bits while engaged, byte 2 = mode + wheel counter, byte 3 =
//! the pressed-button flash). The HOST polls that readback (~50 Hz),
//! decodes the events and writes the mixer registers:
//!
//! - MIX monitoring (cap_mix.pcap): MIX is a TOGGLE — the first press
//!   (byte3 0x44 flash) makes the host write `0x17 0x8480 0x8C80`,
//!   engaging fader mode (the device latches it: engaged bits + byte2
//!   0x00+n counter persist until the exit writes); the second press
//!   writes `0x17 0x0400 0x8000` + `0x8080`, leaving fader mode. The
//!   wheel then adjusts the selected input→output crosspoint (0x12 on
//!   the standard map).
//! - OUT wheel (cap_set2.pcap): the output master fader
//!   (`0x12 0x03E0+2·out` + `0x1A 0x0004+2·out`), ~0.5 dB/click.
//! - IN wheel (cap_panel.pcap): the selected channel's gain.
//!
//! Button flashes (byte3): 0x41=IN, 0x48=OUT, 0x42=SET, 0x44=MIX,
//! 0x50=SELECT, 0x60=DIM. OUT selection = byte1 & 0x07 (0x04=Ch1/2,
//! 0x05=Phones, 0x06=Opt); IN selection = byte2 & 0xF0 base
//! (0x40=Ch1/2, 0x50=Ch3/4, 0x60=Opt); DIM = byte1 bit 0x20.

/// Panel state decoded from the `0x17` status readback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PanelState {
    /// byte0 & 0x80 — the panel is engaged (a button held / MIX active).
    pub engaged: bool,
    /// byte1 & 0x07 — OUT selection: 0x04 = Ch1/2, 0x05 = Phones, 0x06 = Opt.
    pub out_pos: u8,
    /// byte1 & 0x20 — DIM active flag.
    pub dim: bool,
    /// byte2 — mode + selection + wheel counter (see module docs).
    pub byte2: u8,
    /// byte3 — the pressed-button flash (0x40 = idle).
    pub button: u8,
}

impl PanelState {
    /// Decode a `0x17` readback payload.
    pub fn decode(st: [u8; 4]) -> Self {
        Self {
            engaged: st[0] & 0x80 != 0,
            out_pos: st[1] & 0x07,
            dim: st[1] & 0x20 != 0,
            byte2: st[2],
            button: st[3],
        }
    }

    /// IN-mode selection from byte2's high nibble: 0x4x = Ch1/2,
    /// 0x5x = Ch3/4, 0x6x = Opt. Returns 0-2 (falls back to 0).
    pub fn in_sel(&self) -> usize {
        match self.byte2 >> 4 {
            0x5 => 1,
            0x6 => 2,
            _ => 0,
        }
    }

    /// OUT-mode selection from byte1: 0x04 = 0, 0x05 = 1, 0x06 = 2.
    pub fn out_sel(&self) -> usize {
        ((self.out_pos & 0x07).saturating_sub(4)).min(2) as usize
    }

    /// True in OUT mode. NOTE: the OUT-mode wheel counter is a FULL
    /// byte that carries 0x8F → 0x90 (cap_set2.pcap shows byte2
    /// 0x90-0x9F while turning the wheel on an output), so both 0x8x
    /// and 0x9x mean OUT.
    pub fn out_mode(&self) -> bool {
        self.mode_class() == 1
    }

    /// Mode class of byte2's high nibble: 0 = fader (MIX, 0x0x),
    /// 1 = OUT (0x8x/0x9x — the counter carries past 0x8F), 2 = IN
    /// (0x4x/0x5x/0x6x). The wheel delta is only meaningful within one
    /// class (a mode switch must not be read as a wheel jump).
    pub fn mode_class(&self) -> u8 {
        match self.byte2 >> 4 {
            0 => 0,
            0x8 | 0x9 => 1,
            _ => 2,
        }
    }

    /// True in MIX (fader) mode — byte2 high nibble = 0x0 (the wheel
    /// counter is the low nibble, 0x00-0x0F).
    ///
    /// NOTE: NOT gated on `engaged` (byte0 0x80). Hardware probe
    /// 2026-08-24: during a MIX press the readback is `0D 0D 41 44` —
    /// the 0x44 flash on byte3 with byte2 still in the current mode
    /// (IN 0x4x); byte2 only becomes 0x0x AFTER the host sends the
    /// MIX ack (0x17 0x8480 0x8C80). Gate on byte2 alone so the driver
    /// can both detect the press (flash) and hold the mode once the
    /// device enters fader mode.
    pub fn mix_mode(&self) -> bool {
        self.byte2 >> 4 == 0x0
    }
}

/// SELECT channel-selection state (IN mode). Each SELECT press cycles
/// left → right → both → none → left (manual §5.1: "Press SELECT
/// several times to step through left, right or both channels").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectState {
    #[default]
    Left = 0,
    Right = 1,
    Both = 2,
    None = 3,
}

/// A front-panel event detected between two polls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelEvent {
    /// MIX monitoring engaged (fader mode — the wheel controls the
    /// selected input→output monitoring level). Sent on the 0x44 flash
    /// rising edge; the device only enters fader mode after the host
    /// acks it.
    MixPressed,
    /// MIX monitoring released (back to gain mode). Sent on the NEXT
    /// 0x44 flash — MIX is a toggle, the physical release of the
    /// button is not the exit signal.
    MixReleased,
    /// The wheel moved by `delta` clicks (signed, ±15).
    Wheel { delta: i8 },
    /// DIM pressed (byte3 0x60 flash).
    DimPressed,
    /// A button flash on byte3 that we don't act on (IN/OUT/SET/SELECT).
    Button { code: u8 },
}

/// Tracks the panel state between polls and produces [`PanelEvent`]s.
#[derive(Debug, Default)]
pub struct PanelDriver {
    last: Option<PanelState>,
    /// True while MIX monitoring is active.
    pub mix_mode: bool,
    /// True once the device has actually been observed in fader mode
    /// (byte2 0x0x) — gates the device-driven MIX exit so a pre-ack
    /// readback (byte2 still 0x4x while the 0x44 flash shows) never
    /// ends MIX before it started.
    saw_fader: bool,
    /// SELECT channel-selection state (IN mode): L/R/both/none.
    pub select: SelectState,
    /// The last IN selection (0-2) seen while NOT in fader mode. The IN
    /// selection lives in byte2's high nibble, which becomes 0x0 while
    /// the MIX-monitoring fader counter is shown — so it's latched here
    /// for the duration of the MIX press.
    pub in_sel: usize,
    /// The last OUT selection (0-2). byte1 keeps the OUT position in
    /// every mode, so this is just the last-seen value.
    pub out_sel: usize,
}

impl PanelDriver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one decoded readback; returns the events since the last poll.
    pub fn feed(&mut self, st: PanelState) -> Vec<PanelEvent> {
        let mut events = Vec::new();
        match self.last {
            None => {
                // First sample: only latch the state, detect MIX if the
                // capture started mid-press (byte2 already 0x0x, or the
                // 0x44 flash showing).
                if st.mix_mode() || st.button == 0x44 {
                    self.mix_mode = true;
                    events.push(PanelEvent::MixPressed);
                }
                self.saw_fader = st.mix_mode();
                self.in_sel = st.in_sel();
                self.out_sel = st.out_sel();
            }
            Some(prev) => {
                // MIX toggles the encoder between the fader (monitoring)
                // and the gain — manual §5.3 "Monitoring – MIX". The
                // device reports each press as a byte3 0x44 flash; the
                // mode is latched HOST-SIDE: the first press's ack
                // (0x17 0x8480 0x8C80) engages fader mode and the
                // device KEEPS it (engaged bits, byte2 0x00+n counter)
                // until the exit writes (0x17 0x0400 0x8000/0x8080) on
                // the second press. The physical release of the button
                // is NOT the exit signal.
                if st.button == 0x44 && prev.button != 0x44 {
                    self.mix_mode = !self.mix_mode;
                    if self.mix_mode {
                        events.push(PanelEvent::MixPressed);
                    } else {
                        events.push(PanelEvent::MixReleased);
                    }
                }
                // Device-driven exit: a mode button (IN/OUT/SET) pressed
                // during MIX makes the device leave fader mode on its own
                // (byte2's high nibble returns to 0x4x/0x8x) — that ends
                // MIX, so the wheel goes back to the gain. Guarded by
                // `saw_fader`: before the host ack lands, byte2 is still
                // 0x4x while the 0x44 flash shows; only a fader exit
                // after the device actually entered fader mode counts.
                if st.mix_mode() {
                    self.saw_fader = true;
                } else if self.mix_mode && self.saw_fader && st.button != 0x44 {
                    self.mix_mode = false;
                    self.saw_fader = false;
                    events.push(PanelEvent::MixReleased);
                }
                // Latch the IN selection while it's readable (byte2
                // high nibble); in fader mode byte2 = 0x00+n, so keep
                // the last value. byte1 keeps the OUT position in every
                // mode.
                if !st.mix_mode() {
                    self.in_sel = st.in_sel();
                }
                self.out_sel = st.out_sel();
                // Wheel: the low nibble of byte2 is the counter in OUT
                // (0x8x/0x9x) and MIX (0x0x) modes — but only when the
                // MODE CLASS is unchanged: a mode switch (e.g. IN 0x4x
                // → fader 0x0x on a MIX press, or the OUT counter
                // carrying 0x8F → 0x90) must not be read as a wheel
                // jump.
                let prev_cnt = prev.byte2 & 0x0F;
                let new_cnt = st.byte2 & 0x0F;
                if st.mode_class() == prev.mode_class() && new_cnt != prev_cnt {
                    let mut d = new_cnt as i8 - prev_cnt as i8;
                    if d > 8 {
                        d -= 16;
                    } else if d < -8 {
                        d += 16;
                    }
                    if d != 0 {
                        events.push(PanelEvent::Wheel { delta: d });
                    }
                }
                // DIM flash on byte3.
                if st.button == 0x60 && prev.button != 0x60 {
                    events.push(PanelEvent::DimPressed);
                }
                // SELECT flash on byte3 cycles the IN channel selection.
                if st.button == 0x50 && prev.button != 0x50 {
                    self.select = match self.select {
                        SelectState::Left => SelectState::Right,
                        SelectState::Right => SelectState::Both,
                        SelectState::Both => SelectState::None,
                        SelectState::None => SelectState::Left,
                    };
                }
                // Any other distinct button flash.
                if st.button != 0x40 && st.button != 0x60 && st.button != prev.button {
                    events.push(PanelEvent::Button { code: st.button });
                }
            }
        }
        self.last = Some(st);
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_out_selection() {
        // cap_buttons2: 0x04 = Ch1/2, 0x05 = Phones, 0x06 = Opt.
        assert_eq!(PanelState::decode([0x0C, 0x04, 0x8A, 0x40]).out_sel(), 0);
        assert_eq!(PanelState::decode([0x0C, 0x05, 0x8A, 0x40]).out_sel(), 1);
        assert_eq!(PanelState::decode([0x0C, 0x06, 0x8A, 0x40]).out_sel(), 2);
    }

    #[test]
    fn decode_in_selection() {
        // cap_mix: byte2 0x4A/0x5A/0x6A = Ch1/2, Ch3/4, Opt.
        assert_eq!(PanelState::decode([0x0C, 0x05, 0x4A, 0x40]).in_sel(), 0);
        assert_eq!(PanelState::decode([0x0C, 0x05, 0x5A, 0x40]).in_sel(), 1);
        assert_eq!(PanelState::decode([0x0C, 0x05, 0x6A, 0x40]).in_sel(), 2);
    }

    #[test]
    fn out_mode_accepts_counter_carry_0x9x() {
        // cap_buttons2 idle OUT = 0x8x; cap_set2 (wheel on OUT) shows
        // byte2 0x90-0x9F — the counter is a full byte that carries
        // past 0x8F. Both classes are OUT mode.
        assert!(PanelState::decode([0x0C, 0x04, 0x8A, 0x40]).out_mode());
        assert!(PanelState::decode([0x0C, 0x04, 0x9A, 0x40]).out_mode());
        assert!(PanelState::decode([0x0C, 0x04, 0x9F, 0x40]).out_mode());
        assert!(!PanelState::decode([0x0C, 0x04, 0x4A, 0x40]).out_mode());
        assert!(!PanelState::decode([0x0C, 0x04, 0x0A, 0x40]).out_mode());
    }

    #[test]
    fn wheel_delta_survives_out_counter_carry() {
        // The OUT counter carries 0x8F -> 0x90: the mode class is
        // unchanged (both OUT), so it's a +1 wheel click, not a mode
        // switch / dropped event.
        let mut drv = PanelDriver::new();
        drv.feed(PanelState::decode([0x0C, 0x04, 0x8F, 0x40]));
        let ev = drv.feed(PanelState::decode([0x0C, 0x04, 0x90, 0x40]));
        assert!(ev.contains(&PanelEvent::Wheel { delta: 1 }), "{ev:?}");
        // And backward across the carry: 0x90 -> 0x8F = -1.
        let mut drv = PanelDriver::new();
        drv.feed(PanelState::decode([0x0C, 0x04, 0x90, 0x40]));
        let ev = drv.feed(PanelState::decode([0x0C, 0x04, 0x8F, 0x40]));
        assert!(ev.contains(&PanelEvent::Wheel { delta: -1 }), "{ev:?}");
    }

    #[test]
    fn mix_toggle_engage_release() {
        let mut drv = PanelDriver::new();
        // Idle.
        assert!(drv
            .feed(PanelState::decode([0x0C, 0x05, 0x8A, 0x40]))
            .is_empty());
        // MIX press #1: the 0x44 flash toggles fader mode ON.
        let ev = drv.feed(PanelState::decode([0x8C, 0x85, 0x0A, 0x44]));
        assert!(ev.contains(&PanelEvent::MixPressed));
        assert!(drv.mix_mode);
        // Still engaged, byte3 back to 0x40. The device STAYS in fader
        // mode after the physical release (byte2 0x0x) — that's what
        // makes MIX a toggle — so nothing fires.
        assert!(drv
            .feed(PanelState::decode([0x8C, 0x85, 0x0A, 0x40]))
            .is_empty());
        assert!(drv.mix_mode);
        // MIX press #2: toggles back to gain mode. The device is still
        // in fader mode when the flash arrives.
        let ev = drv.feed(PanelState::decode([0x8C, 0x85, 0x0A, 0x44]));
        assert!(ev.contains(&PanelEvent::MixReleased));
        assert!(!drv.mix_mode);
        // After the release acks the device returns to IN mode (0x49)
        // — no further events.
        assert!(drv
            .feed(PanelState::decode([0x0C, 0x05, 0x49, 0x40]))
            .is_empty());
        assert!(!drv.mix_mode);
    }

    #[test]
    fn wheel_delta_wraps() {
        let mut drv = PanelDriver::new();
        drv.feed(PanelState::decode([0x8C, 0x85, 0x0B, 0x40]));
        let ev = drv.feed(PanelState::decode([0x8C, 0x85, 0x0C, 0x40]));
        assert!(ev.contains(&PanelEvent::Wheel { delta: 1 }));
        // Wrap 0x0F -> 0x00 = +1.
        drv.feed(PanelState::decode([0x8C, 0x85, 0x0F, 0x40]));
        let ev = drv.feed(PanelState::decode([0x8C, 0x85, 0x00, 0x40]));
        assert!(ev.contains(&PanelEvent::Wheel { delta: 1 }));
        // Backward 0x02 -> 0x01 = -1.
        drv.feed(PanelState::decode([0x8C, 0x85, 0x02, 0x40]));
        let ev = drv.feed(PanelState::decode([0x8C, 0x85, 0x01, 0x40]));
        assert!(ev.contains(&PanelEvent::Wheel { delta: -1 }));
    }

    #[test]
    fn select_cycles_left_right_both_none() {
        let mut drv = PanelDriver::new();
        drv.feed(PanelState::decode([0x0C, 0x05, 0x4A, 0x40]));
        assert_eq!(drv.select, SelectState::Left);
        // SELECT press: byte3 flash 0x50 (one press = one step).
        let ev = drv.feed(PanelState::decode([0x0C, 0x05, 0x4A, 0x50]));
        assert!(ev.contains(&PanelEvent::Button { code: 0x50 }));
        assert_eq!(drv.select, SelectState::Right);
        drv.feed(PanelState::decode([0x0C, 0x05, 0x4A, 0x40]));
        drv.feed(PanelState::decode([0x0C, 0x05, 0x4A, 0x50]));
        assert_eq!(drv.select, SelectState::Both);
        drv.feed(PanelState::decode([0x0C, 0x05, 0x4A, 0x40]));
        drv.feed(PanelState::decode([0x0C, 0x05, 0x4A, 0x50]));
        assert_eq!(drv.select, SelectState::None);
        drv.feed(PanelState::decode([0x0C, 0x05, 0x4A, 0x40]));
        drv.feed(PanelState::decode([0x0C, 0x05, 0x4A, 0x50]));
        assert_eq!(drv.select, SelectState::Left);
    }

    #[test]
    fn mix_press_detected_from_flash_before_ack() {
        // Hardware probe 2026-08-24: the RAW MIX press readback is
        // `0D 0D 41 44` — byte3 flash 0x44, but byte0 has NO engaged
        // bit and byte2 is still in IN mode (0x41). The old code gated
        // MixPressed on engaged + byte2 0x0x and never fired.
        let mut drv = PanelDriver::new();
        drv.feed(PanelState::decode([0x0C, 0x0D, 0x41, 0x40]));
        let ev = drv.feed(PanelState::decode([0x0D, 0x0D, 0x41, 0x44]));
        assert!(ev.contains(&PanelEvent::MixPressed));
        assert!(drv.mix_mode);
        // Still held (flash persists, byte2 not yet in fader mode
        // because the host ack hasn't landed): NO release yet.
        assert!(drv
            .feed(PanelState::decode([0x0D, 0x0D, 0x41, 0x44]))
            .is_empty());
        // Ack landed: byte2 flips to the fader counter 0x0x.
        assert!(drv
            .feed(PanelState::decode([0x8C, 0x85, 0x0A, 0x40]))
            .is_empty());
        // Release: the NEXT 0x44 flash toggles back to gain mode.
        let ev = drv.feed(PanelState::decode([0x8C, 0x85, 0x0A, 0x44]));
        assert!(ev.contains(&PanelEvent::MixReleased));
        assert!(!drv.mix_mode);
    }

    #[test]
    fn mix_latches_in_selection_across_fader_mode() {
        // The IN selection lives in byte2's high nibble, which becomes
        // 0x0 in fader mode — the driver must remember it.
        let mut drv = PanelDriver::new();
        drv.feed(PanelState::decode([0x0C, 0x05, 0x5A, 0x40])); // Ch3/4
        assert_eq!(drv.in_sel, 1);
        assert_eq!(drv.out_sel, 1); // byte1 0x05 = Phones
        let ev = drv.feed(PanelState::decode([0x0D, 0x05, 0x5A, 0x44])); // MIX
        assert!(ev.contains(&PanelEvent::MixPressed));
        // In fader mode the selection is not readable from byte2.
        drv.feed(PanelState::decode([0x8C, 0x85, 0x00, 0x40]));
        assert_eq!(drv.in_sel, 1);
        assert_eq!(drv.out_sel, 1);
        // Wheel in fader mode must keep using the latched selection.
        drv.feed(PanelState::decode([0x8C, 0x85, 0x01, 0x40]));
        assert!(drv.mix_mode);
        // Second MIX press: back to gain mode.
        let ev = drv.feed(PanelState::decode([0x8C, 0x85, 0x01, 0x44]));
        assert!(ev.contains(&PanelEvent::MixReleased));
        assert!(!drv.mix_mode);
        // Back in IN mode, the selection is readable again.
        drv.feed(PanelState::decode([0x0C, 0x05, 0x59, 0x40])); // Ch3/4
        assert_eq!(drv.in_sel, 1);
        drv.feed(PanelState::decode([0x0C, 0x05, 0x6A, 0x40])); // Opt
        assert_eq!(drv.in_sel, 2);
    }

    #[test]
    fn mix_held_flash_toggles_once() {
        // A single physical press may keep the 0x44 flash across
        // several polls — it must toggle ON exactly once.
        let mut drv = PanelDriver::new();
        drv.feed(PanelState::decode([0x0C, 0x0D, 0x41, 0x40]));
        let ev = drv.feed(PanelState::decode([0x0D, 0x0D, 0x41, 0x44]));
        assert!(ev.contains(&PanelEvent::MixPressed));
        for _ in 0..3 {
            assert!(drv
                .feed(PanelState::decode([0x0D, 0x0D, 0x41, 0x44]))
                .is_empty());
        }
        assert!(drv.mix_mode);
        // Flash clears, then a NEW press toggles OFF.
        drv.feed(PanelState::decode([0x0D, 0x0D, 0x41, 0x40]));
        let ev = drv.feed(PanelState::decode([0x0D, 0x0D, 0x41, 0x44]));
        assert!(ev.contains(&PanelEvent::MixReleased));
        assert!(!drv.mix_mode);
    }

    #[test]
    fn mix_pre_ack_readback_does_not_release() {
        // Before the host ack lands, byte2 is still 0x4x with the 0x44
        // flash showing — the driver must NOT treat that as a fader
        // exit (it has never seen fader mode yet).
        let mut drv = PanelDriver::new();
        drv.feed(PanelState::decode([0x0C, 0x0D, 0x41, 0x40]));
        let ev = drv.feed(PanelState::decode([0x0D, 0x0D, 0x41, 0x44]));
        assert!(ev.contains(&PanelEvent::MixPressed));
        // Flash clears but the ack hasn't taken effect: byte2 still IN.
        assert!(drv
            .feed(PanelState::decode([0x0D, 0x0D, 0x41, 0x40]))
            .is_empty());
        assert!(drv.mix_mode);
        // Ack lands: byte2 -> fader counter.
        assert!(drv
            .feed(PanelState::decode([0x8C, 0x85, 0x02, 0x40]))
            .is_empty());
        assert!(drv.mix_mode);
    }

    #[test]
    fn in_press_during_mix_exits_fader_mode() {
        // Pressing IN while MIX is engaged makes the device leave fader
        // mode by itself (byte2 0x0x -> 0x4A) — that ends MIX (the user:
        // IN should return to gain control).
        let mut drv = PanelDriver::new();
        drv.feed(PanelState::decode([0x0C, 0x05, 0x41, 0x40]));
        let ev = drv.feed(PanelState::decode([0x0D, 0x05, 0x41, 0x44]));
        assert!(ev.contains(&PanelEvent::MixPressed));
        // Ack landed: in fader mode.
        drv.feed(PanelState::decode([0x8C, 0x85, 0x03, 0x40]));
        assert!(drv.mix_mode);
        // IN press: byte2 -> 0x4A (IN mode), flash 0x41.
        let ev = drv.feed(PanelState::decode([0x0C, 0x05, 0x4A, 0x41]));
        assert!(ev.contains(&PanelEvent::MixReleased));
        assert!(!drv.mix_mode);
        // The IN flash itself is also reported as a button event.
        assert!(ev.contains(&PanelEvent::Button { code: 0x41 }));
    }

    #[test]
    fn set_press_emits_button_event() {
        let mut drv = PanelDriver::new();
        drv.feed(PanelState::decode([0x0C, 0x05, 0x4A, 0x40]));
        let ev = drv.feed(PanelState::decode([0x0C, 0x05, 0x4A, 0x42]));
        assert!(ev.contains(&PanelEvent::Button { code: 0x42 }));
    }
}
