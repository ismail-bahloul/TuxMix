# Class Compliant mode — ALSA control reference

Captured 2026-09-12 from the real RME Babyface Pro FS switched into
**Class Compliant mode** (USB `2a39:3fb0`, handled by stock
`snd-usb-audio`), card name `RME Babyface Pro (<serial>)`.

Why this file exists: TuxMix's ALSA backend targets our own
`snd-usb-babyface-pro` driver's control grammar (`Mic 1`, `AN1`,
`Phantom Power Mic 1`, …), *not* this one. Capturing this dump needs
the card physically switched out of proprietary mode, so it is kept
here rather than re-derived. It is the reference for scoping any
future real CC-mode support (see `BabyfacePro::open`'s
`UnsupportedDeviceMode` guard, which currently rejects this mode).

314 simple controls. Grammar: `<Type>-<Name>-<Output>` crosspoints
(`Line-IN3-AN1`), `<Type>-<Name> <Fn>` per-channel (`Mic-AN1 48V`,
`Mic-AN1 Gain`, `Line-IN3 Sens.`), `Main-Out <Out>` masters.

```
ALSA Card Scanner
  Filter: Pro<serial>

--- Card #0  Pro<serial> ---
    longname: RME Babyface Pro (<serial>) at usb-<bus-path>, high speed
    driver:   USB-Audio

    VOL  PCM-ADAT3-ADAT3                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT3-ADAT4                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT3-ADAT5                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT3-ADAT6                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT3-ADAT7                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT3-ADAT8                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT3-AN1                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT3-AN2                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT3-AS1                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT3-AS2                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT3-PH3                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT3-PH4                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT4-ADAT3                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT4-ADAT4                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT4-ADAT5                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT4-ADAT6                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT4-ADAT7                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT4-ADAT8                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT4-AN1                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT4-AN2                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT4-AS1                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT4-AS2                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT4-PH3                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT4-PH4                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT5-ADAT3                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT5-ADAT4                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT5-ADAT5                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT5-ADAT6                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT5-ADAT7                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT5-ADAT8                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT5-AN1                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT5-AN2                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT5-AS1                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT5-AS2                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT5-PH3                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT5-PH4                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT6-ADAT3                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT6-ADAT4                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT6-ADAT5                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT6-ADAT6                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT6-ADAT7                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT6-ADAT8                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT6-AN1                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT6-AN2                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT6-AS1                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT6-AS2                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT6-PH3                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT6-PH4                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT7-ADAT3                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT7-ADAT4                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT7-ADAT5                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT7-ADAT6                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT7-ADAT7                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT7-ADAT8                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT7-AN1                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT7-AN2                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT7-AS1                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT7-AS2                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT7-PH3                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT7-PH4                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT8-ADAT3                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT8-ADAT4                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT8-ADAT5                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT8-ADAT6                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT8-ADAT7                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT8-ADAT8                 value=0       range=[0-65536]  0%
    VOL  PCM-ADAT8-AN1                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT8-AN2                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT8-AS1                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT8-AS2                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT8-PH3                   value=0       range=[0-65536]  0%
    VOL  PCM-ADAT8-PH4                   value=0       range=[0-65536]  0%
    VOL  PCM-AN1-ADAT3                   value=0       range=[0-65536]  0%
    VOL  PCM-AN1-ADAT4                   value=0       range=[0-65536]  0%
    VOL  PCM-AN1-ADAT5                   value=0       range=[0-65536]  0%
    VOL  PCM-AN1-ADAT6                   value=0       range=[0-65536]  0%
    VOL  PCM-AN1-ADAT7                   value=0       range=[0-65536]  0%
    VOL  PCM-AN1-ADAT8                   value=0       range=[0-65536]  0%
    VOL  PCM-AN1-AN1                     value=1       range=[0-65536]  0%
    VOL  PCM-AN1-AN2                     value=1       range=[0-65536]  0%
    VOL  PCM-AN1-AS1                     value=0       range=[0-65536]  0%
    VOL  PCM-AN1-AS2                     value=0       range=[0-65536]  0%
    VOL  PCM-AN1-PH3                     value=65536   range=[0-65536]  100%
    VOL  PCM-AN1-PH4                     value=0       range=[0-65536]  0%
    VOL  PCM-AN2-ADAT3                   value=0       range=[0-65536]  0%
    VOL  PCM-AN2-ADAT4                   value=0       range=[0-65536]  0%
    VOL  PCM-AN2-ADAT5                   value=0       range=[0-65536]  0%
    VOL  PCM-AN2-ADAT6                   value=0       range=[0-65536]  0%
    VOL  PCM-AN2-ADAT7                   value=0       range=[0-65536]  0%
    VOL  PCM-AN2-ADAT8                   value=0       range=[0-65536]  0%
    VOL  PCM-AN2-AN1                     value=1       range=[0-65536]  0%
    VOL  PCM-AN2-AN2                     value=1       range=[0-65536]  0%
    VOL  PCM-AN2-AS1                     value=0       range=[0-65536]  0%
    VOL  PCM-AN2-AS2                     value=0       range=[0-65536]  0%
    VOL  PCM-AN2-PH3                     value=0       range=[0-65536]  0%
    VOL  PCM-AN2-PH4                     value=65536   range=[0-65536]  100%
    VOL  PCM-AS1-ADAT3                   value=0       range=[0-65536]  0%
    VOL  PCM-AS1-ADAT4                   value=0       range=[0-65536]  0%
    VOL  PCM-AS1-ADAT5                   value=0       range=[0-65536]  0%
    VOL  PCM-AS1-ADAT6                   value=0       range=[0-65536]  0%
    VOL  PCM-AS1-ADAT7                   value=0       range=[0-65536]  0%
    VOL  PCM-AS1-ADAT8                   value=0       range=[0-65536]  0%
    VOL  PCM-AS1-AN1                     value=1       range=[0-65536]  0%
    VOL  PCM-AS1-AN2                     value=1       range=[0-65536]  0%
    VOL  PCM-AS1-AS1                     value=0       range=[0-65536]  0%
    VOL  PCM-AS1-AS2                     value=0       range=[0-65536]  0%
    VOL  PCM-AS1-PH3                     value=0       range=[0-65536]  0%
    VOL  PCM-AS1-PH4                     value=0       range=[0-65536]  0%
    VOL  PCM-AS2-ADAT3                   value=0       range=[0-65536]  0%
    VOL  PCM-AS2-ADAT4                   value=0       range=[0-65536]  0%
    VOL  PCM-AS2-ADAT5                   value=0       range=[0-65536]  0%
    VOL  PCM-AS2-ADAT6                   value=0       range=[0-65536]  0%
    VOL  PCM-AS2-ADAT7                   value=0       range=[0-65536]  0%
    VOL  PCM-AS2-ADAT8                   value=0       range=[0-65536]  0%
    VOL  PCM-AS2-AN1                     value=1       range=[0-65536]  0%
    VOL  PCM-AS2-AN2                     value=1       range=[0-65536]  0%
    VOL  PCM-AS2-AS1                     value=0       range=[0-65536]  0%
    VOL  PCM-AS2-AS2                     value=0       range=[0-65536]  0%
    VOL  PCM-AS2-PH3                     value=0       range=[0-65536]  0%
    VOL  PCM-AS2-PH4                     value=0       range=[0-65536]  0%
    VOL  PCM-PH3-ADAT3                   value=0       range=[0-65536]  0%
    VOL  PCM-PH3-ADAT4                   value=0       range=[0-65536]  0%
    VOL  PCM-PH3-ADAT5                   value=0       range=[0-65536]  0%
    VOL  PCM-PH3-ADAT6                   value=0       range=[0-65536]  0%
    VOL  PCM-PH3-ADAT7                   value=0       range=[0-65536]  0%
    VOL  PCM-PH3-ADAT8                   value=0       range=[0-65536]  0%
    VOL  PCM-PH3-AN1                     value=1       range=[0-65536]  0%
    VOL  PCM-PH3-AN2                     value=1       range=[0-65536]  0%
    VOL  PCM-PH3-AS1                     value=0       range=[0-65536]  0%
    VOL  PCM-PH3-AS2                     value=0       range=[0-65536]  0%
    VOL  PCM-PH3-PH3                     value=0       range=[0-65536]  0%
    VOL  PCM-PH3-PH4                     value=0       range=[0-65536]  0%
    VOL  PCM-PH4-ADAT3                   value=0       range=[0-65536]  0%
    VOL  PCM-PH4-ADAT4                   value=0       range=[0-65536]  0%
    VOL  PCM-PH4-ADAT5                   value=0       range=[0-65536]  0%
    VOL  PCM-PH4-ADAT6                   value=0       range=[0-65536]  0%
    VOL  PCM-PH4-ADAT7                   value=0       range=[0-65536]  0%
    VOL  PCM-PH4-ADAT8                   value=0       range=[0-65536]  0%
    VOL  PCM-PH4-AN1                     value=1       range=[0-65536]  0%
    VOL  PCM-PH4-AN2                     value=1       range=[0-65536]  0%
    VOL  PCM-PH4-AS1                     value=0       range=[0-65536]  0%
    VOL  PCM-PH4-AS2                     value=0       range=[0-65536]  0%
    VOL  PCM-PH4-PH3                     value=0       range=[0-65536]  0%
    VOL  PCM-PH4-PH4                     value=0       range=[0-65536]  0%
    VOL  Line-ADAT3-ADAT3                value=0       range=[0-65536]  0%
    VOL  Line-ADAT3-ADAT4                value=0       range=[0-65536]  0%
    VOL  Line-ADAT3-ADAT5                value=0       range=[0-65536]  0%
    VOL  Line-ADAT3-ADAT6                value=0       range=[0-65536]  0%
    VOL  Line-ADAT3-ADAT7                value=0       range=[0-65536]  0%
    VOL  Line-ADAT3-ADAT8                value=0       range=[0-65536]  0%
    VOL  Line-ADAT3-AN1                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT3-AN2                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT3-AS1                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT3-AS2                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT3-PH3                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT3-PH4                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT4-ADAT3                value=0       range=[0-65536]  0%
    VOL  Line-ADAT4-ADAT4                value=0       range=[0-65536]  0%
    VOL  Line-ADAT4-ADAT5                value=0       range=[0-65536]  0%
    VOL  Line-ADAT4-ADAT6                value=0       range=[0-65536]  0%
    VOL  Line-ADAT4-ADAT7                value=0       range=[0-65536]  0%
    VOL  Line-ADAT4-ADAT8                value=0       range=[0-65536]  0%
    VOL  Line-ADAT4-AN1                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT4-AN2                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT4-AS1                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT4-AS2                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT4-PH3                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT4-PH4                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT5-ADAT3                value=0       range=[0-65536]  0%
    VOL  Line-ADAT5-ADAT4                value=0       range=[0-65536]  0%
    VOL  Line-ADAT5-ADAT5                value=0       range=[0-65536]  0%
    VOL  Line-ADAT5-ADAT6                value=0       range=[0-65536]  0%
    VOL  Line-ADAT5-ADAT7                value=0       range=[0-65536]  0%
    VOL  Line-ADAT5-ADAT8                value=0       range=[0-65536]  0%
    VOL  Line-ADAT5-AN1                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT5-AN2                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT5-AS1                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT5-AS2                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT5-PH3                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT5-PH4                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT6-ADAT3                value=0       range=[0-65536]  0%
    VOL  Line-ADAT6-ADAT4                value=0       range=[0-65536]  0%
    VOL  Line-ADAT6-ADAT5                value=0       range=[0-65536]  0%
    VOL  Line-ADAT6-ADAT6                value=0       range=[0-65536]  0%
    VOL  Line-ADAT6-ADAT7                value=0       range=[0-65536]  0%
    VOL  Line-ADAT6-ADAT8                value=0       range=[0-65536]  0%
    VOL  Line-ADAT6-AN1                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT6-AN2                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT6-AS1                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT6-AS2                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT6-PH3                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT6-PH4                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT7-ADAT3                value=0       range=[0-65536]  0%
    VOL  Line-ADAT7-ADAT4                value=0       range=[0-65536]  0%
    VOL  Line-ADAT7-ADAT5                value=0       range=[0-65536]  0%
    VOL  Line-ADAT7-ADAT6                value=0       range=[0-65536]  0%
    VOL  Line-ADAT7-ADAT7                value=0       range=[0-65536]  0%
    VOL  Line-ADAT7-ADAT8                value=0       range=[0-65536]  0%
    VOL  Line-ADAT7-AN1                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT7-AN2                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT7-AS1                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT7-AS2                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT7-PH3                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT7-PH4                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT8-ADAT3                value=0       range=[0-65536]  0%
    VOL  Line-ADAT8-ADAT4                value=0       range=[0-65536]  0%
    VOL  Line-ADAT8-ADAT5                value=0       range=[0-65536]  0%
    VOL  Line-ADAT8-ADAT6                value=0       range=[0-65536]  0%
    VOL  Line-ADAT8-ADAT7                value=0       range=[0-65536]  0%
    VOL  Line-ADAT8-ADAT8                value=0       range=[0-65536]  0%
    VOL  Line-ADAT8-AN1                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT8-AN2                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT8-AS1                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT8-AS2                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT8-PH3                  value=0       range=[0-65536]  0%
    VOL  Line-ADAT8-PH4                  value=0       range=[0-65536]  0%
    VOL  Line-AS1-ADAT3                  value=0       range=[0-65536]  0%
    VOL  Line-AS1-ADAT4                  value=0       range=[0-65536]  0%
    VOL  Line-AS1-ADAT5                  value=0       range=[0-65536]  0%
    VOL  Line-AS1-ADAT6                  value=0       range=[0-65536]  0%
    VOL  Line-AS1-ADAT7                  value=0       range=[0-65536]  0%
    VOL  Line-AS1-ADAT8                  value=0       range=[0-65536]  0%
    VOL  Line-AS1-AN1                    value=0       range=[0-65536]  0%
    VOL  Line-AS1-AN2                    value=0       range=[0-65536]  0%
    VOL  Line-AS1-AS1                    value=0       range=[0-65536]  0%
    VOL  Line-AS1-AS2                    value=0       range=[0-65536]  0%
    VOL  Line-AS1-PH3                    value=0       range=[0-65536]  0%
    VOL  Line-AS1-PH4                    value=0       range=[0-65536]  0%
    VOL  Line-AS2-ADAT3                  value=0       range=[0-65536]  0%
    VOL  Line-AS2-ADAT4                  value=0       range=[0-65536]  0%
    VOL  Line-AS2-ADAT5                  value=0       range=[0-65536]  0%
    VOL  Line-AS2-ADAT6                  value=0       range=[0-65536]  0%
    VOL  Line-AS2-ADAT7                  value=0       range=[0-65536]  0%
    VOL  Line-AS2-ADAT8                  value=0       range=[0-65536]  0%
    VOL  Line-AS2-AN1                    value=0       range=[0-65536]  0%
    VOL  Line-AS2-AN2                    value=0       range=[0-65536]  0%
    VOL  Line-AS2-AS1                    value=0       range=[0-65536]  0%
    VOL  Line-AS2-AS2                    value=0       range=[0-65536]  0%
    VOL  Line-AS2-PH3                    value=0       range=[0-65536]  0%
    VOL  Line-AS2-PH4                    value=0       range=[0-65536]  0%
    VOL  Line-IN3-ADAT3                  value=0       range=[0-65536]  0%
    VOL  Line-IN3-ADAT4                  value=0       range=[0-65536]  0%
    VOL  Line-IN3-ADAT5                  value=0       range=[0-65536]  0%
    VOL  Line-IN3-ADAT6                  value=0       range=[0-65536]  0%
    VOL  Line-IN3-ADAT7                  value=0       range=[0-65536]  0%
    VOL  Line-IN3-ADAT8                  value=0       range=[0-65536]  0%
    VOL  Line-IN3-AN1                    value=92      range=[0-65536]  0%
    VOL  Line-IN3-AN2                    value=92      range=[0-65536]  0%
    VOL  Line-IN3-AS1                    value=0       range=[0-65536]  0%
    VOL  Line-IN3-AS2                    value=0       range=[0-65536]  0%
    VOL  Line-IN3-PH3                    value=0       range=[0-65536]  0%
    VOL  Line-IN3-PH4                    value=0       range=[0-65536]  0%
    VOL  Line-IN4-ADAT3                  value=0       range=[0-65536]  0%
    VOL  Line-IN4-ADAT4                  value=0       range=[0-65536]  0%
    VOL  Line-IN4-ADAT5                  value=0       range=[0-65536]  0%
    VOL  Line-IN4-ADAT6                  value=0       range=[0-65536]  0%
    VOL  Line-IN4-ADAT7                  value=0       range=[0-65536]  0%
    VOL  Line-IN4-ADAT8                  value=0       range=[0-65536]  0%
    VOL  Line-IN4-AN1                    value=0       range=[0-65536]  0%
    VOL  Line-IN4-AN2                    value=0       range=[0-65536]  0%
    VOL  Line-IN4-AS1                    value=0       range=[0-65536]  0%
    VOL  Line-IN4-AS2                    value=0       range=[0-65536]  0%
    VOL  Line-IN4-PH3                    value=0       range=[0-65536]  0%
    VOL  Line-IN4-PH4                    value=0       range=[0-65536]  0%
    VOL  Line-IN3 Gain                   value=0       range=[0-18]  0%
    ENUM Line-IN3 Sens.                  selected=0  (2 items)
    VOL  Line-IN4 Gain                   value=0       range=[0-18]  0%
    ENUM Line-IN4 Sens.                  selected=0  (2 items)
    VOL  Mic-AN1-ADAT3                   value=0       range=[0-65536]  0%
    VOL  Mic-AN1-ADAT4                   value=0       range=[0-65536]  0%
    VOL  Mic-AN1-ADAT5                   value=0       range=[0-65536]  0%
    VOL  Mic-AN1-ADAT6                   value=0       range=[0-65536]  0%
    VOL  Mic-AN1-ADAT7                   value=0       range=[0-65536]  0%
    VOL  Mic-AN1-ADAT8                   value=0       range=[0-65536]  0%
    VOL  Mic-AN1-AN1                     value=1       range=[0-65536]  0%
    VOL  Mic-AN1-AN2                     value=1       range=[0-65536]  0%
    VOL  Mic-AN1-AS1                     value=0       range=[0-65536]  0%
    VOL  Mic-AN1-AS2                     value=0       range=[0-65536]  0%
    VOL  Mic-AN1-PH3                     value=0       range=[0-65536]  0%
    VOL  Mic-AN1-PH4                     value=0       range=[0-65536]  0%
    VOL  Mic-AN2-ADAT3                   value=0       range=[0-65536]  0%
    VOL  Mic-AN2-ADAT4                   value=0       range=[0-65536]  0%
    VOL  Mic-AN2-ADAT5                   value=0       range=[0-65536]  0%
    VOL  Mic-AN2-ADAT6                   value=0       range=[0-65536]  0%
    VOL  Mic-AN2-ADAT7                   value=0       range=[0-65536]  0%
    VOL  Mic-AN2-ADAT8                   value=0       range=[0-65536]  0%
    VOL  Mic-AN2-AN1                     value=1       range=[0-65536]  0%
    VOL  Mic-AN2-AN2                     value=0       range=[0-65536]  0%
    VOL  Mic-AN2-AS1                     value=0       range=[0-65536]  0%
    VOL  Mic-AN2-AS2                     value=0       range=[0-65536]  0%
    VOL  Mic-AN2-PH3                     value=0       range=[0-65536]  0%
    VOL  Mic-AN2-PH4                     value=0       range=[0-65536]  0%
    SW   Mic-AN1 48V                     ON
    VOL  Mic-AN1 Gain                    value=33      range=[0-65]  51%
    SW   Mic-AN1 PAD                     OFF
    SW   Mic-AN2 48V                     OFF
    VOL  Mic-AN2 Gain                    value=0       range=[0-65]  0%
    SW   Mic-AN2 PAD                     OFF
    SW   IEC958                          OFF
    SW   IEC958 Emphasis                 OFF
    SW   IEC958 Pro Mask                 OFF
    VOL  Main-Out ADAT3                  value=1       range=[0-65536]  0%
    VOL  Main-Out ADAT4                  value=1       range=[0-65536]  0%
    VOL  Main-Out ADAT5                  value=1       range=[0-65536]  0%
    VOL  Main-Out ADAT6                  value=1       range=[0-65536]  0%
    VOL  Main-Out ADAT7                  value=1       range=[0-65536]  0%
    VOL  Main-Out ADAT8                  value=1       range=[0-65536]  0%
    VOL  Main-Out AN1                    value=1       range=[0-65536]  0%
    VOL  Main-Out AN2                    value=1       range=[0-65536]  0%
    VOL  Main-Out AS1                    value=1       range=[0-65536]  0%
    VOL  Main-Out AS2                    value=1       range=[0-65536]  0%
    VOL  Main-Out PH3                    value=1       range=[0-65536]  0%
    VOL  Main-Out PH4                    value=1       range=[0-65536]  0%
    ENUM Sample Clock Source             selected=0  (2 items)
    -- 314 element(s) total --

```
