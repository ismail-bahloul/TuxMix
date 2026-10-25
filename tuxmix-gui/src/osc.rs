//! OSC (Open Sound Control) control surface.
//!
//! Lets an external tool — a hardware controller, a script, a second UI —
//! read and write mixer state over UDP alongside the GUI. Opt-in via
//! `--osc` (see `main.rs`); off by default, and bound to loopback only, so
//! enabling it never silently exposes control to the LAN.
//!
//! Address space (native, v1 — matrix channels only, matching what
//! `tuxmix-core` actually exposes today):
//! - `/input/<id>/volume/<out>` f   `/playback/<id>/volume/<out>` f
//! - `/input/<id>/pan/<out>` f      `/playback/<id>/pan/<out>` f  (-100..100)
//! - `/input|playback|output/<id>/mute` f|i|T|F (0/1)
//! - `/input|playback|output/<id>/solo` f|i|T|F (0/1)
//! - `/output/<id>/volume` f
//!
//! `--osc-dialect rme` swaps this for RME's documented TotalMix OSC
//! address space, the one oscmix speaks too (see `Dialect::Rme`), so both
//! TotalMix clients and oscmix UIs can drive a Babyface through TuxMix.
//! Indices are 0-based there, volumes are dB, pan is -1.0..+1.0, and a
//! crosspoint is `/mix/{in|pb}/<source>/<submix>/{fader,faderlin,balpan,solo}`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use iced::futures::channel::mpsc;
use iced::futures::{SinkExt, Stream, StreamExt};
use iced::stream;
use rosc::{OscMessage, OscPacket, OscType};
use tokio::net::UdpSocket;

use tuxmix_core::ChannelId;

use crate::app::Message;

/// Which address space the bridge speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Dialect {
    /// TuxMix's own v1 addresses, documented above.
    #[default]
    Native,
    /// RME's TotalMix dialect, the one oscmix speaks: 0-based indices,
    /// volumes in dB, pan in -1.0..+1.0, and separate crosspoint parameters
    /// under `/mix/{in|pb}/<source>/<submix>/`. A TuxMix output pair is two
    /// consecutive mono submixes (`2p`, `2p+1`).
    Rme,
}

impl Dialect {
    pub fn from_arg(s: &str) -> Option<Self> {
        match s {
            "native" => Some(Dialect::Native),
            // oscmix uses RME's addressing, so the old name keeps working.
            "rme" | "oscmix" => Some(Dialect::Rme),
            _ => None,
        }
    }
}

/// Bind/target ports for the OSC bridge — `Hash` because it's threaded
/// through `Subscription::run_with` as the recipe's identifying data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OscConfig {
    pub recv_port: u16,
    pub send_port: u16,
    pub send_host: IpAddr,
    pub dialect: Dialect,
}

/// A control command decoded from an incoming OSC message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OscCommand {
    Volume(ChannelId, usize, f32),
    Pan(ChannelId, usize, i8),
    Mute(ChannelId, bool),
    Solo(ChannelId, bool),
    OutputVolume(usize, f32),
    /// oscmix `/refresh` — re-send the whole state.
    Refresh,
}

/// A state change to echo out to any connected OSC client — sent both for
/// GUI-originated changes and for changes that came in as an `OscCommand`,
/// so a controller and the GUI never drift out of sync with each other.
#[derive(Debug, Clone, PartialEq)]
pub enum OscOutbound {
    Volume(ChannelId, usize, f32),
    Pan(ChannelId, usize, i8),
    Mute(ChannelId, bool),
    Solo(ChannelId, bool),
    OutputVolume(usize, f32),
    /// Device identity, sent on connect/refresh. Only the oscmix dialect has
    /// an address for it.
    DeviceInfo {
        name: String,
        inputs: usize,
        outputs: usize,
    },
}

fn channel_prefix(id: ChannelId) -> (&'static str, usize) {
    match id {
        ChannelId::Input(i) => ("input", i),
        ChannelId::Playback(i) => ("playback", i),
        ChannelId::Output(i) => ("output", i),
    }
}

/// input/playback/output — everywhere mute/solo are addressable.
fn parse_any_channel(kind: &str, id: usize) -> Option<ChannelId> {
    match kind {
        "input" => Some(ChannelId::Input(id)),
        "playback" => Some(ChannelId::Playback(id)),
        "output" => Some(ChannelId::Output(id)),
        _ => None,
    }
}

/// input/playback only — outputs have no per-output-pair matrix routing
/// (`OutputChannel` is a single scalar volume, not a `Vec` per pair), so
/// `/output/<id>/volume/<out>` is deliberately not a valid address.
fn parse_matrix_channel(kind: &str, id: usize) -> Option<ChannelId> {
    match kind {
        "input" => Some(ChannelId::Input(id)),
        "playback" => Some(ChannelId::Playback(id)),
        _ => None,
    }
}

fn arg_f32(v: &OscType) -> Option<f32> {
    match v {
        OscType::Float(f) => Some(*f),
        OscType::Double(d) => Some(*d as f32),
        OscType::Int(i) => Some(*i as f32),
        _ => None,
    }
}

fn arg_bool(v: &OscType) -> Option<bool> {
    match v {
        OscType::Bool(b) => Some(*b),
        OscType::Int(i) => Some(*i != 0),
        OscType::Float(f) => Some(*f >= 0.5),
        _ => None,
    }
}

impl OscCommand {
    pub fn parse(addr: &str, args: &[OscType]) -> Option<Self> {
        let parts: Vec<&str> = addr.split('/').filter(|s| !s.is_empty()).collect();
        match parts.as_slice() {
            [kind, id, "volume", out] => {
                let id: usize = id.parse().ok()?;
                let out: usize = out.parse().ok()?;
                let v = arg_f32(args.first()?)?;
                Some(OscCommand::Volume(
                    parse_matrix_channel(kind, id)?,
                    out,
                    v.clamp(0.0, 2.0),
                ))
            }
            [kind, id, "pan", out] => {
                let id: usize = id.parse().ok()?;
                let out: usize = out.parse().ok()?;
                let v = arg_f32(args.first()?)?.clamp(-100.0, 100.0);
                Some(OscCommand::Pan(
                    parse_matrix_channel(kind, id)?,
                    out,
                    v as i8,
                ))
            }
            [kind, id, "mute"] => {
                let id: usize = id.parse().ok()?;
                let v = arg_bool(args.first()?)?;
                Some(OscCommand::Mute(parse_any_channel(kind, id)?, v))
            }
            [kind, id, "solo"] => {
                let id: usize = id.parse().ok()?;
                let v = arg_bool(args.first()?)?;
                Some(OscCommand::Solo(parse_any_channel(kind, id)?, v))
            }
            ["output", id, "volume"] => {
                let id: usize = id.parse().ok()?;
                let v = arg_f32(args.first()?)?;
                Some(OscCommand::OutputVolume(id, v.clamp(0.0, 2.0)))
            }
            _ => None,
        }
    }
}

/// RME's fader curve (sheet "Fader curve"): a position of 0..1 maps to
/// -65 dB .. +6 dB, and anything below -64.9 dB means off.
const FADER_MAX_POS: f32 = 1023.0;
const RME_OFF_DB: f32 = -64.9;

fn fader_pos_to_db(pos: f32) -> f32 {
    let p = pos.clamp(0.0, 1.0) * FADER_MAX_POS;
    let db = if p >= 649.0 {
        p * 0.032_085_5615 - 26.823_529_4118
    } else {
        p * p * (-1.0 / 11033.0) + p * 0.149_732_6203 - 65.0
    };
    if db < RME_OFF_DB {
        f32::NEG_INFINITY
    } else {
        db
    }
}

/// Inverse of `fader_pos_to_db`, used when reporting a fader position.
#[allow(dead_code)]
fn db_to_fader_pos(db: f32) -> f32 {
    let p = if db >= -6.0 {
        (db + 26.823_529_4118) / 0.032_085_5615
    } else {
        826.0 - (-34869.0 - 11033.0 * db).max(0.0).sqrt()
    };
    (p / FADER_MAX_POS).clamp(0.0, 1.0)
}

/// RME fader values are dB, with anything below -64.9 dB meaning silence.
fn db_to_linear(db: f32) -> f32 {
    if db < RME_OFF_DB {
        0.0
    } else {
        10f32.powf(db / 20.0).clamp(0.0, 2.0)
    }
}

fn linear_to_db(v: f32) -> f32 {
    if v <= 0.0 {
        -300.0
    } else {
        (20.0 * v.log10()).clamp(-300.0, 6.0)
    }
}

/// RME's `/mix` bus names: inputs and playbacks are sources.
fn mix_source(bus: &str, channel: &str) -> Option<ChannelId> {
    let id: usize = channel.parse().ok()?;
    match bus {
        "in" => Some(ChannelId::Input(id)),
        "pb" => Some(ChannelId::Playback(id)),
        _ => None,
    }
}

/// Per-channel addresses. Inputs, playbacks and outputs are all mono
/// channels here; only a `/mix` submix is a pair (see `osc_snapshot`).
fn bus_channel(kind: &str, channel: &str) -> Option<ChannelId> {
    let id: usize = channel.parse().ok()?;
    match kind {
        "input" => Some(ChannelId::Input(id)),
        "playback" => Some(ChannelId::Playback(id)),
        "output" => Some(ChannelId::Output(id)),
        _ => None,
    }
}

impl OscCommand {
    /// Decodes an incoming message in RME's TotalMix dialect, which oscmix
    /// speaks too. Indices are 0-based; every crosspoint parameter is its
    /// own address, so a message carries at most one command.
    pub fn parse_rme(addr: &str, args: &[OscType]) -> Vec<Self> {
        let parts: Vec<&str> = addr.split('/').filter(|s| !s.is_empty()).collect();
        let first = args.first();
        match parts.as_slice() {
            // oscmix's /refresh is not RME, but it is the name every
            // existing client uses to ask for the full state.
            ["refresh"] | ["sendall"] | ["sendsettings"] | ["sendmix"] | ["sendstate"] => {
                vec![OscCommand::Refresh]
            }
            ["sendchan", _, _] | ["sendsubmix", _] => vec![OscCommand::Refresh],
            ["mix", bus, src, out, param] => {
                let (Some(cid), Some(out)) = (mix_source(bus, src), out.parse::<usize>().ok())
                else {
                    return vec![];
                };
                let pair = out / 2;
                match *param {
                    "fader" => match first.and_then(arg_f32) {
                        Some(db) => vec![OscCommand::Volume(cid, pair, db_to_linear(db))],
                        None => vec![],
                    },
                    "faderlin" => match first.and_then(arg_f32) {
                        Some(pos) => vec![OscCommand::Volume(
                            cid,
                            pair,
                            db_to_linear(fader_pos_to_db(pos)),
                        )],
                        None => vec![],
                    },
                    "balpan" => match first.and_then(arg_f32) {
                        Some(p) => vec![OscCommand::Pan(
                            cid,
                            pair,
                            (p.clamp(-1.0, 1.0) * 100.0).round() as i8,
                        )],
                        None => vec![],
                    },
                    "solo" => match first.and_then(arg_bool) {
                        Some(s) => vec![OscCommand::Solo(cid, s)],
                        None => vec![],
                    },
                    _ => vec![],
                }
            }
            [kind, channel, "mute"] => match (bus_channel(kind, channel), first.and_then(arg_bool))
            {
                (Some(cid), Some(v)) => vec![OscCommand::Mute(cid, v)],
                _ => vec![],
            },
            ["output", channel, "volume"] => {
                let (Some(id), Some(db)) = (channel.parse::<usize>().ok(), first.and_then(arg_f32))
                else {
                    return vec![];
                };
                vec![OscCommand::OutputVolume(id, db_to_linear(db))]
            }
            _ => vec![],
        }
    }
}

impl OscOutbound {
    /// Encodes this change for the RME dialect. `mono_outputs` is how many
    /// mono output channels the device reports; a TuxMix solo is per source
    /// across every submix, so it fans out to that many `/mix` nodes.
    fn into_rme_packets(self, mono_outputs: usize) -> Vec<OscPacket> {
        let msg = |addr: String, args: Vec<OscType>| OscPacket::Message(OscMessage { addr, args });
        let mix = |id: ChannelId, out: usize, param: &str, value: OscType| -> Option<OscPacket> {
            let (bus, i) = match id {
                ChannelId::Input(i) => ("in", i),
                ChannelId::Playback(i) => ("pb", i),
                ChannelId::Output(_) => return None,
            };
            Some(msg(
                format!("/mix/{bus}/{i}/{}/{param}", out * 2),
                vec![value],
            ))
        };
        match self {
            OscOutbound::Volume(id, out, v) => {
                mix(id, out, "fader", OscType::Float(linear_to_db(v)))
                    .into_iter()
                    .collect()
            }
            OscOutbound::Pan(id, out, p) => {
                mix(id, out, "balpan", OscType::Float(f32::from(p) / 100.0))
                    .into_iter()
                    .collect()
            }
            OscOutbound::Mute(id, m) => {
                let (kind, i) = channel_prefix(id);
                let value = OscType::Float(if m { 1.0 } else { 0.0 });
                vec![msg(format!("/{kind}/{i}/mute"), vec![value])]
            }
            OscOutbound::Solo(id, s) => {
                let (bus, i) = match id {
                    ChannelId::Input(i) => ("in", i),
                    ChannelId::Playback(i) => ("pb", i),
                    ChannelId::Output(_) => return Vec::new(),
                };
                let value = OscType::Float(if s { 1.0 } else { 0.0 });
                // One node per stereo pair: RME addresses the even channel of
                // a pair, the same way fader and balpan do.
                (0..mono_outputs / 2)
                    .map(|pair| {
                        msg(
                            format!("/mix/{bus}/{i}/{}/solo", pair * 2),
                            vec![value.clone()],
                        )
                    })
                    .collect()
            }
            OscOutbound::OutputVolume(id, v) => vec![msg(
                format!("/output/{id}/volume"),
                vec![OscType::Float(linear_to_db(v))],
            )],
            OscOutbound::DeviceInfo {
                name,
                inputs,
                outputs,
            } => vec![
                msg("/status/device".into(), vec![OscType::String(name.clone())]),
                msg("/status/connection".into(), vec![OscType::Float(1.0)]),
                msg("/status/dsp".into(), vec![OscType::Float(0.0)]),
                msg("/device/name".into(), vec![OscType::String(name.clone())]),
                msg("/device/uid".into(), vec![OscType::String(name)]),
                msg("/device/flags".into(), vec![OscType::Int(0)]),
                msg("/device/inputs".into(), vec![OscType::Int(inputs as i32)]),
                msg("/device/outputs".into(), vec![OscType::Int(outputs as i32)]),
            ],
        }
    }
}

impl OscOutbound {
    fn into_packet(self) -> Option<OscPacket> {
        let (addr, args) = match self {
            OscOutbound::Volume(id, out, v) => {
                let (kind, i) = channel_prefix(id);
                (format!("/{kind}/{i}/volume/{out}"), vec![OscType::Float(v)])
            }
            OscOutbound::Pan(id, out, p) => {
                let (kind, i) = channel_prefix(id);
                (
                    format!("/{kind}/{i}/pan/{out}"),
                    vec![OscType::Float(p as f32)],
                )
            }
            OscOutbound::Mute(id, m) => {
                let (kind, i) = channel_prefix(id);
                (
                    format!("/{kind}/{i}/mute"),
                    vec![OscType::Float(if m { 1.0 } else { 0.0 })],
                )
            }
            OscOutbound::Solo(id, s) => {
                let (kind, i) = channel_prefix(id);
                (
                    format!("/{kind}/{i}/solo"),
                    vec![OscType::Float(if s { 1.0 } else { 0.0 })],
                )
            }
            OscOutbound::OutputVolume(id, v) => {
                (format!("/output/{id}/volume"), vec![OscType::Float(v)])
            }
            OscOutbound::DeviceInfo { .. } => return None,
        };
        Some(OscPacket::Message(OscMessage { addr, args }))
    }
}

/// One human-readable line per OSC message crossing the bridge in either
/// direction — for `Message::OscLog`/the debug log panel in `app.rs`. Built
/// from the raw address + args, *before* `OscCommand::parse` gets a chance
/// to reject anything — so a controller sending a malformed or unmapped
/// address (e.g. a typo, or a channel index the device doesn't have) still
/// shows up here, which is the actual point of a debug log: seeing why
/// nothing happened, not just confirming when something did.
fn log_line(dir: &str, addr: &str, args: &[OscType]) -> String {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = elapsed.as_secs() % 86400;
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    format!(
        "{h:02}:{m:02}:{s:02}.{:03} {dir} {addr} {args:?}",
        elapsed.subsec_millis()
    )
}

/// Runs the OSC bridge as an iced `Subscription` recipe: binds a UDP
/// socket on loopback, decodes incoming packets into `Message::OscCommand`,
/// and drains an internal channel (handed back to the app via
/// `Message::OscReady`) to send outgoing feedback packets. Every message in
/// either direction also gets echoed out as a `Message::OscLog` line, for
/// the debug log panel — a no-op from the bridge's own perspective if that
/// panel is never opened, since the log is a plain bounded ring buffer on
/// the `update()` side, not something this worker needs to know about.
///
/// Never touches `state.device` — the `alsa` mixer handle it wraps has no
/// `Send`/`Sync` impl, so it must stay owned by the single update-loop
/// thread. This worker only ever exchanges plain values with `update()`,
/// the same way the existing global keyboard/mouse listener does.
pub fn worker(config: &OscConfig) -> impl Stream<Item = Message> {
    let config = *config;
    stream::channel(100, async move |mut output| {
        let recv_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), config.recv_port);
        let socket = match UdpSocket::bind(recv_addr).await {
            Ok(s) => s,
            Err(err) => {
                log::error!("OSC: failed to bind {recv_addr}: {err}");
                return;
            }
        };
        let send_addr = SocketAddr::new(config.send_host, config.send_port);

        // A full snapshot is several hundred messages, and it is pushed in
        // one tight loop. 100 slots silently dropped the tail of it.
        let (tx, mut rx) = mpsc::channel::<OscOutbound>(1024);
        if output.send(Message::OscReady(tx)).await.is_err() {
            return;
        }

        log::info!("OSC: listening on {recv_addr}, feedback to {send_addr}");
        let mut buf = [0u8; rosc::decoder::MTU];
        // Filled from DeviceInfo on connect. Only used to fan a per-channel
        // solo out over every submix node; 12 is the Babyface Pro FS.
        let mut mono_outputs = 12usize;
        loop {
            tokio::select! {
                res = socket.recv_from(&mut buf) => {
                    let Ok((size, _from)) = res else { continue };
                    let Ok((_, packet)) = rosc::decoder::decode_udp(&buf[..size]) else { continue };
                    if let OscPacket::Message(msg) = packet {
                        let _ = output
                            .send(Message::OscLog(log_line("IN ", &msg.addr, &msg.args)))
                            .await;
                        let cmds = match config.dialect {
                            Dialect::Native => {
                                OscCommand::parse(&msg.addr, &msg.args).into_iter().collect()
                            }
                            Dialect::Rme => OscCommand::parse_rme(&msg.addr, &msg.args),
                        };
                        for cmd in cmds {
                            if output.send(Message::OscCommand(cmd)).await.is_err() {
                                return;
                            }
                        }
                    }
                }
                outbound = rx.next() => {
                    let Some(outbound) = outbound else { continue };
                    let packets = match config.dialect {
                        Dialect::Native => outbound.into_packet().into_iter().collect(),
                        Dialect::Rme => {
                            if let OscOutbound::DeviceInfo { outputs, .. } = &outbound {
                                mono_outputs = *outputs;
                            }
                            outbound.into_rme_packets(mono_outputs)
                        }
                    };
                    for packet in packets {
                        if let OscPacket::Message(ref msg) = packet {
                            let _ = output
                                .send(Message::OscLog(log_line("OUT", &msg.addr, &msg.args)))
                                .await;
                        }
                        if let Ok(bytes) = rosc::encoder::encode(&packet) {
                            let _ = socket.send_to(&bytes, send_addr).await;
                        }
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_matrix_volume() {
        let cmd = OscCommand::parse("/input/2/volume/0", &[OscType::Float(0.75)]);
        assert_eq!(cmd, Some(OscCommand::Volume(ChannelId::Input(2), 0, 0.75)));
    }

    #[test]
    fn parses_playback_pan_with_int_arg() {
        let cmd = OscCommand::parse("/playback/1/pan/3", &[OscType::Int(-50)]);
        assert_eq!(cmd, Some(OscCommand::Pan(ChannelId::Playback(1), 3, -50)));
    }

    #[test]
    fn parses_mute_bool_and_float_forms() {
        assert_eq!(
            OscCommand::parse("/input/0/mute", &[OscType::Bool(true)]),
            Some(OscCommand::Mute(ChannelId::Input(0), true))
        );
        assert_eq!(
            OscCommand::parse("/output/5/mute", &[OscType::Float(1.0)]),
            Some(OscCommand::Mute(ChannelId::Output(5), true))
        );
    }

    #[test]
    fn parses_solo() {
        let cmd = OscCommand::parse("/input/4/solo", &[OscType::Int(1)]);
        assert_eq!(cmd, Some(OscCommand::Solo(ChannelId::Input(4), true)));
    }

    #[test]
    fn parses_output_volume() {
        let cmd = OscCommand::parse("/output/1/volume", &[OscType::Float(0.5)]);
        assert_eq!(cmd, Some(OscCommand::OutputVolume(1, 0.5)));
    }

    #[test]
    fn rejects_output_matrix_address() {
        // Outputs have no per-output-pair routing — `/output/<id>/volume/<out>`
        // is not a valid address, unlike inputs/playbacks.
        assert_eq!(
            OscCommand::parse("/output/1/volume/0", &[OscType::Float(0.5)]),
            None
        );
    }

    #[test]
    fn rejects_unknown_and_malformed_addresses() {
        assert_eq!(
            OscCommand::parse("/input/2/gain/0", &[OscType::Float(0.5)]),
            None
        );
        assert_eq!(
            OscCommand::parse("/input/volume/0", &[OscType::Float(0.5)]),
            None
        );
        assert_eq!(OscCommand::parse("/input/2/volume/0", &[]), None);
        assert_eq!(
            OscCommand::parse("/bogus/2/volume/0", &[OscType::Float(0.5)]),
            None
        );
    }

    #[test]
    fn outbound_round_trips_through_encode_decode() {
        let packet = OscOutbound::Volume(ChannelId::Input(3), 1, 0.42)
            .into_packet()
            .unwrap();
        let bytes = rosc::encoder::encode(&packet).unwrap();
        let (_, decoded) = rosc::decoder::decode_udp(&bytes).unwrap();
        let OscPacket::Message(msg) = decoded else {
            panic!("expected a message");
        };
        assert_eq!(msg.addr, "/input/3/volume/1");
        let cmd = OscCommand::parse(&msg.addr, &msg.args);
        assert_eq!(cmd, Some(OscCommand::Volume(ChannelId::Input(3), 1, 0.42)));
    }

    fn float_arg(packet: &OscPacket) -> f32 {
        let OscPacket::Message(m) = packet else {
            panic!("expected a message")
        };
        match &m.args[0] {
            OscType::Float(f) => *f,
            other => panic!("expected a float, got {other:?}"),
        }
    }

    #[test]
    fn rme_mix_fader_is_zero_based_db() {
        assert_eq!(
            OscCommand::parse_rme("/mix/in/0/0/fader", &[OscType::Float(-6.0)]),
            vec![OscCommand::Volume(
                ChannelId::Input(0),
                0,
                db_to_linear(-6.0)
            )]
        );
        assert_eq!(
            OscCommand::parse_rme("/mix/pb/1/2/fader", &[OscType::Float(-6.0)]),
            vec![OscCommand::Volume(
                ChannelId::Playback(1),
                1,
                db_to_linear(-6.0)
            )]
        );
    }

    #[test]
    fn rme_fader_below_the_floor_is_silence() {
        assert_eq!(
            OscCommand::parse_rme("/mix/in/3/0/fader", &[OscType::Float(-300.0)]),
            vec![OscCommand::Volume(ChannelId::Input(3), 0, 0.0)]
        );
    }

    #[test]
    fn rme_faderlin_uses_the_fader_curve() {
        let cmds = OscCommand::parse_rme("/mix/in/0/0/faderlin", &[OscType::Float(1.0)]);
        let OscCommand::Volume(ChannelId::Input(0), 0, v) = cmds[0] else {
            panic!("expected a volume")
        };
        assert!((v - db_to_linear(6.0)).abs() < 1e-4, "got {v}");

        assert_eq!(
            OscCommand::parse_rme("/mix/in/0/0/faderlin", &[OscType::Float(0.0)]),
            vec![OscCommand::Volume(ChannelId::Input(0), 0, 0.0)]
        );
    }

    #[test]
    fn rme_balpan_is_minus_one_to_one() {
        assert_eq!(
            OscCommand::parse_rme("/mix/in/2/3/balpan", &[OscType::Float(0.5)]),
            vec![OscCommand::Pan(ChannelId::Input(2), 1, 50)]
        );
        assert_eq!(
            OscCommand::parse_rme("/mix/in/2/3/balpan", &[OscType::Float(-1.0)]),
            vec![OscCommand::Pan(ChannelId::Input(2), 1, -100)]
        );
    }

    #[test]
    fn rme_mix_solo() {
        assert_eq!(
            OscCommand::parse_rme("/mix/in/4/0/solo", &[OscType::Float(1.0)]),
            vec![OscCommand::Solo(ChannelId::Input(4), true)]
        );
    }

    #[test]
    fn rme_channel_mute_and_output_volume() {
        assert_eq!(
            OscCommand::parse_rme("/input/0/mute", &[OscType::Float(1.0)]),
            vec![OscCommand::Mute(ChannelId::Input(0), true)]
        );
        assert_eq!(
            OscCommand::parse_rme("/output/3/mute", &[OscType::Float(1.0)]),
            vec![OscCommand::Mute(ChannelId::Output(3), true)]
        );
        assert_eq!(
            OscCommand::parse_rme("/output/2/volume", &[OscType::Float(-6.0)]),
            vec![OscCommand::OutputVolume(2, db_to_linear(-6.0))]
        );
    }

    #[test]
    fn rme_dump_requests_trigger_a_snapshot() {
        for addr in [
            "/refresh",
            "/sendall",
            "/sendsettings",
            "/sendmix",
            "/sendstate",
            "/sendchan/in/2",
            "/sendsubmix/0",
        ] {
            assert_eq!(
                OscCommand::parse_rme(addr, &[]),
                vec![OscCommand::Refresh],
                "{addr}"
            );
        }
    }

    #[test]
    fn rme_rejects_unknown_bus_and_malformed() {
        assert!(OscCommand::parse_rme("/mix/out/0/0/fader", &[OscType::Float(-6.0)]).is_empty());
        assert!(OscCommand::parse_rme("/mix/in/x/0/fader", &[OscType::Float(-6.0)]).is_empty());
        assert!(OscCommand::parse_rme("/input/-1/mute", &[OscType::Float(1.0)]).is_empty());
    }

    #[test]
    fn rme_outbound_uses_separate_fader_and_balpan() {
        let packets = OscOutbound::Volume(ChannelId::Input(0), 0, 1.0).into_rme_packets(6);
        let OscPacket::Message(m) = &packets[0] else {
            panic!("expected a message")
        };
        assert_eq!(m.addr, "/mix/in/0/0/fader");
        assert!(float_arg(&packets[0]).abs() < 1e-4);

        let packets = OscOutbound::Pan(ChannelId::Input(0), 1, 25).into_rme_packets(6);
        let OscPacket::Message(m) = &packets[0] else {
            panic!("expected a message")
        };
        assert_eq!(m.addr, "/mix/in/0/2/balpan");
        assert_eq!(m.args, vec![OscType::Float(0.25)]);
    }

    #[test]
    fn rme_outbound_fans_solo_over_every_submix() {
        let packets = OscOutbound::Solo(ChannelId::Playback(1), true).into_rme_packets(6);
        assert_eq!(packets.len(), 3);
        let OscPacket::Message(m) = &packets[1] else {
            panic!("expected a message")
        };
        assert_eq!(m.addr, "/mix/pb/1/2/solo");
        assert_eq!(m.args, vec![OscType::Float(1.0)]);
    }

    #[test]
    fn rme_outbound_mute_and_output_volume_use_mono_indices() {
        let packets = OscOutbound::Mute(ChannelId::Output(1), true).into_rme_packets(6);
        let OscPacket::Message(m) = &packets[0] else {
            panic!("expected a message")
        };
        assert_eq!(m.addr, "/output/1/mute");

        let packets = OscOutbound::OutputVolume(1, 0.5).into_rme_packets(6);
        let OscPacket::Message(m) = &packets[0] else {
            panic!("expected a message")
        };
        assert_eq!(m.addr, "/output/1/volume");
        assert!((float_arg(&packets[0]) + 6.0206).abs() < 1e-3);
    }

    #[test]
    fn rme_device_info_serves_status_and_device() {
        let packets = OscOutbound::DeviceInfo {
            name: "Babyface Pro FS".into(),
            inputs: 12,
            outputs: 6,
        }
        .into_rme_packets(6);
        let addrs: Vec<&str> = packets
            .iter()
            .filter_map(|p| match p {
                OscPacket::Message(m) => Some(m.addr.as_str()),
                _ => None,
            })
            .collect();
        assert!(addrs.contains(&"/status/device"));
        assert!(addrs.contains(&"/status/connection"));
        assert!(addrs.contains(&"/device/inputs"));
    }
}
