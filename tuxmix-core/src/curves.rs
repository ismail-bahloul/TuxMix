//! Calibrated hardware curves shared by every backend that needs a
//! dB<->raw conversion for the crosspoint-fader family of registers
//! (crosspoint volumes, FX send). Kept feature-independent (no `alsa`/
//! `usb` deps) so both `babyface.rs` and `usb.rs` can use the exact
//! same calibration without duplicating the table or, worse, drifting
//! apart with two independently-tuned copies.

/// Calibrated crosspoint-fader curve (AN1→AN1/2, cap_calib.pcap
/// 2026-08-22). The user swept the fader +6 → -inf in ~1-dB steps;
/// `0x0000` = -inf (digital mute), `0x0003` = -62 dB, … `0x2D41` =
/// +6 dB. Cross-checked against the scene-load capture: -20 dB =
/// 0x0243. The bottom is ±1 dB uncertain (-62 vs -63); the top and the
/// -20 dB anchor are user/independently confirmed.
pub(crate) const FADER_CURVE: &[(f32, u16)] = &[
    (-62.0, 0x0003),
    (-61.0, 0x0004),
    (-60.0, 0x0005),
    (-59.0, 0x0006),
    (-58.0, 0x0007),
    (-57.0, 0x0008),
    (-56.0, 0x0009),
    (-55.0, 0x000A),
    (-54.0, 0x000B),
    (-53.0, 0x000D),
    (-52.0, 0x000E),
    (-51.0, 0x0010),
    (-50.0, 0x0012),
    (-49.0, 0x0014),
    (-48.0, 0x0017),
    (-47.0, 0x0019),
    (-46.0, 0x001D),
    (-45.0, 0x0020),
    (-44.0, 0x0024),
    (-43.0, 0x0029),
    (-42.0, 0x002E),
    (-41.0, 0x0033),
    (-40.0, 0x003A),
    (-39.0, 0x0041),
    (-38.0, 0x0049),
    (-37.0, 0x0051),
    (-36.0, 0x005B),
    (-35.0, 0x0067),
    (-34.0, 0x0073),
    (-33.0, 0x0081),
    (-32.0, 0x0091),
    (-31.0, 0x00A3),
    (-30.0, 0x00B7),
    (-29.0, 0x00CD),
    (-28.0, 0x00E6),
    (-27.0, 0x0102),
    (-26.0, 0x0122),
    (-25.0, 0x0145),
    (-24.0, 0x016D),
    (-23.0, 0x019A),
    (-22.0, 0x01CC),
    (-21.0, 0x0204),
    (-20.0, 0x0243),
    (-19.0, 0x028A),
    (-18.0, 0x02D9),
    (-17.0, 0x0332),
    (-16.0, 0x0396),
    (-15.0, 0x0406),
    (-14.0, 0x0483),
    (-13.0, 0x0510),
    (-12.0, 0x05AF),
    (-11.0, 0x0660),
    (-10.0, 0x0727),
    (-9.0, 0x0807),
    (-8.0, 0x0902),
    (-7.0, 0x0A1B),
    (-6.0, 0x0B57),
    (-5.0, 0x0CB9),
    (-4.0, 0x0E47),
    (-3.0, 0x1004),
    (-2.0, 0x11F9),
    (-1.0, 0x142A),
    (0.0, 0x16A0),
    (1.0, 0x1963),
    (2.0, 0x1C7C),
    (3.0, 0x1FF6),
    (4.0, 0x23DC),
    (5.0, 0x283D),
    (6.0, 0x2D41),
];
/// Fader raw value for "muted / -inf" (digital mute).
pub(crate) const FADER_MUTE_RAW: u16 = 0x0000;

/// Calibrated: dB → crosspoint-fader raw (linear interpolation).
/// Below the table's bottom → mute; above +6 dB → clamp.
pub(crate) fn fader_db_to_raw(db: f32) -> u16 {
    let (db0, raw0) = FADER_CURVE[0];
    let (dbn, rawn) = *FADER_CURVE.last().unwrap();
    if db <= db0 {
        return if db < db0 { FADER_MUTE_RAW } else { raw0 };
    }
    if db >= dbn {
        return rawn;
    }
    for w in FADER_CURVE.windows(2) {
        let (a_db, a_raw) = w[0];
        let (b_db, b_raw) = w[1];
        if db >= a_db && db <= b_db {
            let t = (db - a_db) / (b_db - a_db);
            return (a_raw as f32 + (b_raw as f32 - a_raw as f32) * t).round() as u16;
        }
    }
    FADER_MUTE_RAW
}

/// Calibrated: crosspoint-fader raw → dB (inverse; raw 0 → -inf).
pub(crate) fn fader_raw_to_db(raw: u16) -> f32 {
    if raw <= FADER_MUTE_RAW || raw < FADER_CURVE[0].1 {
        return -65.0; // -inf
    }
    let (dbn, rawn) = *FADER_CURVE.last().unwrap();
    if raw >= rawn {
        return dbn;
    }
    for w in FADER_CURVE.windows(2) {
        let (a_db, a_raw) = w[0];
        let (b_db, b_raw) = w[1];
        if raw >= a_raw && raw <= b_raw {
            let t = (raw - a_raw) as f32 / (b_raw - a_raw) as f32;
            return a_db + (b_db - a_db) * t;
        }
    }
    -65.0
}
