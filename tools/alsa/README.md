# TuxMix ALSA/PipeWire bridge

Real system audio on the RME Babyface Pro FS in **proprietary mode**
(TotalMix protocol) — the big missing piece before this commit.

## Architecture

```
ALSA app (aplay, DAW, PipeWire…)
   │  snd_pcm_writei/readi (S24_3LE, 4 ch)
   ▼
libasound_module_pcm_tuxmix.so   (snd_pcm_ioplug plugin)
   │  tuxmix_sys.h C ABI
   ▼
libtuxmix_sys.so   (tuxmix-sys, Rust cdylib)
   │  BabyfaceAudio: rings (1 s) + libusb pump thread + capture wake fd
   ▼
tuxmix-usb   (the RE'd proprietary protocol: interface 5, ep 0x01/0x82,
             14×32-bit frames, 24-bit audio in bytes 1-3)
```

- **Playback**: 4 channels = PB1 (OUT frame ch0/1) + PB2 (ch2/3) into
  the TotalMix mixer — route them to an output with the TuxMix
  GUI/TUI (or `cargo run -p tuxmix-usb --features driver --example
  route_pb` for PB1+PB2 → Phones @ -20 dB).
- **Capture**: 4 channels = AN1-4 (IN frame ch0-3). The IN markers
  (ch4/5) and the ADAT/SPDIF channels (ch6-13) are not exposed yet.
- The mixer registers persist on the device, so routing set by the GUI
  stays when the plugin streams.

## Build + install

```bash
cargo build -p tuxmix-sys --release   # libtuxmix_sys.so
sudo make -C tools/alsa install       # plugin + lib + /etc/alsa/conf.d/50-tuxmix.conf
```

The plugin loads `libtuxmix_sys.so` from `target/release` (rpath) for
local testing; `make install` copies it to `/usr/lib`.

## Usage

```bash
# playback (a 4 s 440 Hz sine — generate it or use any S24_3LE file)
aplay  -D tuxmix -f S24_3LE -c 4 -r 48000 sine.wav

# capture (talk into AN1; the file's ch0 = AN1)
arecord -D tuxmix -f S24_3LE -c 4 -r 48000 -d 4 rec.wav

# route PB1+PB2 to the Phones (the mixer registers persist)
cargo run -p tuxmix-usb --features driver --example route_pb
```

PipeWire apps can use it by making `pcm.tuxmix` the default
(`pcm.!default tuxmix` in `~/.asoundrc`, or a PipeWire virtual ALSA
device).

## PipeWire (validated 2026-08-24, pw-play sine by ear)

PipeWire exposes the card as a stereo sink + source via a
`pipewire.conf.d` drop-in (`tools/alsa/50-tuxmix-pipewire.conf`, install
to `~/.config/pipewire/pipewire.conf.d/`):

```bash
cp tools/alsa/50-tuxmix-pipewire.conf ~/.config/pipewire/pipewire.conf.d/50-tuxmix.conf
systemctl --user restart pipewire pipewire-pulse
pactl set-default-sink tuxmix
```

Notes:
- Uses the PipeWire 1.6 spa-alsa factories `api.alsa.pcm.sink` /
  `api.alsa.pcm.source` (the old `pcm.playback`/`pcm.capture` names no
  longer exist in libspa-alsa — and `libpipewire-module-alsa` is gone
  entirely, don't use it).
- `context.objects` appends to the stock list (arrays merge), so the
  Dummy-Driver etc. stay.
- The plugin accepts 2 or 4 channels; PipeWire's stereo upmix lands on
  PB1 (ch0/1), 4-channel streams use PB1+PB2.
- The sink and the source share ONE streaming session per process
  (tuxmix-sys singleton + idempotent start) — the device has a single
  session.

## Known limits (V1)

- 4 channels in/out (AN1-4 / PB1-2); ADAT/SPDIF and the FX-return
  channels not exposed yet.
- The mixer is NOT driven by the plugin — use the GUI/TUI.
- Only one stream client at a time (the GUI must be closed while the
  plugin runs — the device has a single streaming session).
- The ioplug buffer is bounded by the 1 s ring; latency ≈ the ALSA
  buffer size (ring/cadence tuning = the CC-beating latency goal).
