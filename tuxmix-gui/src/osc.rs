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

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use iced::futures::channel::mpsc;
use iced::futures::{SinkExt, Stream, StreamExt};
use iced::stream;
use rosc::{OscMessage, OscPacket, OscType};
use tokio::net::UdpSocket;

use tuxmix_core::ChannelId;

use crate::app::Message;

/// Bind/target ports for the OSC bridge — `Hash` because it's threaded
/// through `Subscription::run_with` as the recipe's identifying data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OscConfig {
    pub recv_port: u16,
    pub send_port: u16,
    pub send_host: IpAddr,
}

/// A control command decoded from an incoming OSC message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OscCommand {
    Volume(ChannelId, usize, f32),
    Pan(ChannelId, usize, i8),
    Mute(ChannelId, bool),
    Solo(ChannelId, bool),
    OutputVolume(usize, f32),
}

/// A state change to echo out to any connected OSC client — sent both for
/// GUI-originated changes and for changes that came in as an `OscCommand`,
/// so a controller and the GUI never drift out of sync with each other.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OscOutbound {
    Volume(ChannelId, usize, f32),
    Pan(ChannelId, usize, i8),
    Mute(ChannelId, bool),
    Solo(ChannelId, bool),
    OutputVolume(usize, f32),
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

impl OscOutbound {
    fn into_packet(self) -> OscPacket {
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
        };
        OscPacket::Message(OscMessage { addr, args })
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
        loop {
            tokio::select! {
                res = socket.recv_from(&mut buf) => {
                    let Ok((size, _from)) = res else { continue };
                    let Ok((_, packet)) = rosc::decoder::decode_udp(&buf[..size]) else { continue };
                    if let OscPacket::Message(msg) = packet {
                        let _ = output
                            .send(Message::OscLog(log_line("IN ", &msg.addr, &msg.args)))
                            .await;
                        if let Some(cmd) = OscCommand::parse(&msg.addr, &msg.args) {
                            if output.send(Message::OscCommand(cmd)).await.is_err() {
                                return;
                            }
                        }
                    }
                }
                outbound = rx.next() => {
                    let Some(outbound) = outbound else { continue };
                    let packet = outbound.into_packet();
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
        let packet = OscOutbound::Volume(ChannelId::Input(3), 1, 0.42).into_packet();
        let bytes = rosc::encoder::encode(&packet).unwrap();
        let (_, decoded) = rosc::decoder::decode_udp(&bytes).unwrap();
        let OscPacket::Message(msg) = decoded else {
            panic!("expected a message");
        };
        assert_eq!(msg.addr, "/input/3/volume/1");
        let cmd = OscCommand::parse(&msg.addr, &msg.args);
        assert_eq!(cmd, Some(OscCommand::Volume(ChannelId::Input(3), 1, 0.42)));
    }
}
