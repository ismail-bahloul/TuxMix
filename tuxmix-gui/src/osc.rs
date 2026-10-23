//! OSC (Open Sound Control) control surface.
//!
//! Lets an external tool — a hardware controller, a script, a second UI —
//! read and write mixer state over UDP alongside the GUI. Opt-in via
//! `--osc` (see `main.rs`); off by default, and bound to loopback only, so
//! enabling it never silently exposes control to the LAN.
//!
//! Address space (v1 — matrix channels only, matching what `tuxmix-core`
//! actually exposes today):
//! - `/input/<id>/volume/<out>` f   `/playback/<id>/volume/<out>` f
//! - `/input/<id>/pan/<out>` f      `/playback/<id>/pan/<out>` f  (-100..100)
//! - `/input|playback|output/<id>/mute` f|i|T|F (0/1)
//! - `/input|playback|output/<id>/solo` f|i|T|F (0/1)
//! - `/output/<id>/volume` f
//!
//! `--osc-dialect oscmix` swaps this for the address space of
//! [oscmix](https://github.com/michaelforney/oscmix) (see `Dialect::Oscmix`),
//! so oscmix UIs and tools can drive a Babyface through TuxMix.

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
    /// oscmix's: 1-based indices, volumes in dB, `/mix/<out>/<input|playback>/<in>`
    /// carrying volume + pan together, `/refresh`, and `/device/*` info.
    /// TuxMix output pairs map to oscmix mono outputs `2p+1`/`2p+2`.
    Oscmix,
}

impl Dialect {
    pub fn from_arg(s: &str) -> Option<Self> {
        match s {
            "native" => Some(Dialect::Native),
            "oscmix" => Some(Dialect::Oscmix),
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

/// oscmix volumes are dB, with anything at or below -65 meaning silence.
fn db_to_linear(db: f32) -> f32 {
    if db <= -65.0 {
        0.0
    } else {
        10f32.powf(db / 20.0).clamp(0.0, 2.0)
    }
}

fn linear_to_db(v: f32) -> f32 {
    if v <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * v.log10()
    }
}

/// oscmix outputs are mono, 1-based; a TuxMix output is a stereo pair.
fn oscmix_out_to_pair(n: usize) -> Option<usize> {
    n.checked_sub(1).map(|i| i / 2)
}

fn pair_to_oscmix_out(pair: usize) -> usize {
    pair * 2 + 1
}

impl OscCommand {
    /// Decodes an incoming message in the oscmix dialect. One message can
    /// carry several commands (`/mix/...` takes volume and pan together).
    pub fn parse_oscmix(addr: &str, args: &[OscType]) -> Vec<Self> {
        let parts: Vec<&str> = addr.split('/').filter(|s| !s.is_empty()).collect();
        let one_based = |s: &str| s.parse::<usize>().ok().filter(|n| *n >= 1);
        match parts.as_slice() {
            ["refresh"] => vec![OscCommand::Refresh],
            ["mix", out, kind, inn] => {
                let (Some(out), Some(inn)) = (one_based(out), one_based(inn)) else {
                    return vec![];
                };
                let (Some(pair), Some(cid)) =
                    (oscmix_out_to_pair(out), parse_matrix_channel(kind, inn - 1))
                else {
                    return vec![];
                };
                let mut cmds = Vec::new();
                let mut rest = args;
                // `N` in the volume slot means "leave the volume alone".
                match rest.first() {
                    Some(OscType::Nil) => rest = &rest[1..],
                    Some(a) => {
                        let Some(db) = arg_f32(a) else { return vec![] };
                        cmds.push(OscCommand::Volume(cid, pair, db_to_linear(db)));
                        rest = &rest[1..];
                    }
                    None => return vec![],
                }
                if let Some(a) = rest.first() {
                    if let Some(p) = arg_f32(a) {
                        cmds.push(OscCommand::Pan(cid, pair, p.clamp(-100.0, 100.0) as i8));
                    }
                }
                cmds
            }
            [kind, n, "mute"] => {
                let (Some(n), Some(v)) = (one_based(n), args.first().and_then(arg_bool)) else {
                    return vec![];
                };
                let id = if *kind == "output" {
                    n.checked_sub(1).map(|i| i / 2)
                } else {
                    n.checked_sub(1)
                };
                match id.and_then(|id| parse_any_channel(kind, id)) {
                    Some(cid) => vec![OscCommand::Mute(cid, v)],
                    None => vec![],
                }
            }
            ["output", n, "volume"] => {
                let (Some(pair), Some(db)) = (
                    one_based(n).and_then(oscmix_out_to_pair),
                    args.first().and_then(arg_f32),
                ) else {
                    return vec![];
                };
                vec![OscCommand::OutputVolume(pair, db_to_linear(db))]
            }
            _ => vec![],
        }
    }
}

/// Last volume/pan sent per crosspoint, so each oscmix `/mix/...` message
/// can carry both values the way oscmix itself emits them.
type MixCache = std::collections::HashMap<(ChannelId, usize), (f32, i8)>;

impl OscOutbound {
    /// Encodes this change for the oscmix dialect — empty when oscmix has
    /// no address for it (solo).
    fn into_oscmix_packets(self, cache: &mut MixCache) -> Vec<OscPacket> {
        let msg = |addr: String, args: Vec<OscType>| OscPacket::Message(OscMessage { addr, args });
        let mix = |id: ChannelId, out: usize, (v, p): (f32, i8)| {
            let (kind, i) = channel_prefix(id);
            msg(
                format!("/mix/{}/{kind}/{}", pair_to_oscmix_out(out), i + 1),
                vec![OscType::Float(linear_to_db(v)), OscType::Int(i32::from(p))],
            )
        };
        match self {
            OscOutbound::Volume(id, out, v) => {
                let e = cache.entry((id, out)).or_insert((v, 0));
                e.0 = v;
                vec![mix(id, out, *e)]
            }
            OscOutbound::Pan(id, out, p) => {
                let e = cache.entry((id, out)).or_insert((1.0, p));
                e.1 = p;
                vec![mix(id, out, *e)]
            }
            OscOutbound::Mute(id, m) => {
                let (kind, i) = channel_prefix(id);
                let n = if kind == "output" {
                    pair_to_oscmix_out(i)
                } else {
                    i + 1
                };
                vec![msg(
                    format!("/{kind}/{n}/mute"),
                    vec![OscType::Int(i32::from(m))],
                )]
            }
            OscOutbound::Solo(..) => vec![],
            OscOutbound::OutputVolume(id, v) => vec![msg(
                format!("/output/{}/volume", pair_to_oscmix_out(id)),
                vec![OscType::Float(linear_to_db(v))],
            )],
            OscOutbound::DeviceInfo {
                name,
                inputs,
                outputs,
            } => vec![
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

        let (tx, mut rx) = mpsc::channel::<OscOutbound>(100);
        if output.send(Message::OscReady(tx)).await.is_err() {
            return;
        }

        log::info!("OSC: listening on {recv_addr}, feedback to {send_addr}");
        let mut buf = [0u8; rosc::decoder::MTU];
        let mut mix_cache = MixCache::new();
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
                            Dialect::Oscmix => OscCommand::parse_oscmix(&msg.addr, &msg.args),
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
                        Dialect::Oscmix => outbound.into_oscmix_packets(&mut mix_cache),
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

    #[test]
    fn oscmix_mix_sets_volume_and_pan() {
        // oscmix out 3 = TuxMix pair 1; input 2 = id 1; 0 dB = 1.0 linear.
        let cmds =
            OscCommand::parse_oscmix("/mix/3/input/2", &[OscType::Float(0.0), OscType::Int(-40)]);
        assert_eq!(
            cmds,
            vec![
                OscCommand::Volume(ChannelId::Input(1), 1, 1.0),
                OscCommand::Pan(ChannelId::Input(1), 1, -40),
            ]
        );
    }

    #[test]
    fn oscmix_mix_nil_volume_keeps_it_and_floor_is_silence() {
        let cmds = OscCommand::parse_oscmix("/mix/1/playback/1", &[OscType::Nil, OscType::Int(10)]);
        assert_eq!(cmds, vec![OscCommand::Pan(ChannelId::Playback(0), 0, 10)]);
        let cmds = OscCommand::parse_oscmix("/mix/1/input/1", &[OscType::Float(-65.0)]);
        assert_eq!(cmds, vec![OscCommand::Volume(ChannelId::Input(0), 0, 0.0)]);
    }

    #[test]
    fn oscmix_rejects_zero_index_and_unknown() {
        assert!(OscCommand::parse_oscmix("/mix/0/input/1", &[OscType::Float(0.0)]).is_empty());
        assert!(OscCommand::parse_oscmix("/input/0/mute", &[OscType::Int(1)]).is_empty());
        assert!(OscCommand::parse_oscmix("/input/1/solo", &[OscType::Int(1)]).is_empty());
    }

    #[test]
    fn oscmix_mute_volume_refresh() {
        assert_eq!(
            OscCommand::parse_oscmix("/output/4/mute", &[OscType::Int(1)]),
            vec![OscCommand::Mute(ChannelId::Output(1), true)]
        );
        assert_eq!(
            OscCommand::parse_oscmix("/output/3/volume", &[OscType::Float(-6.0)]),
            vec![OscCommand::OutputVolume(1, db_to_linear(-6.0))]
        );
        assert_eq!(
            OscCommand::parse_oscmix("/refresh", &[]),
            vec![OscCommand::Refresh]
        );
    }

    #[test]
    fn oscmix_outbound_combines_volume_and_pan() {
        let mut cache = MixCache::new();
        let id = ChannelId::Input(0);
        OscOutbound::Volume(id, 0, 1.0).into_oscmix_packets(&mut cache);
        let pkts = OscOutbound::Pan(id, 0, 25).into_oscmix_packets(&mut cache);
        let OscPacket::Message(m) = &pkts[0] else {
            panic!()
        };
        assert_eq!(m.addr, "/mix/1/input/1");
        assert_eq!(m.args, vec![OscType::Float(0.0), OscType::Int(25)]);
        assert!(OscOutbound::Solo(id, true)
            .into_oscmix_packets(&mut cache)
            .is_empty());
    }
}
