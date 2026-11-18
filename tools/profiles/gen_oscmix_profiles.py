#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# Copyright (c) 2026 Ismaïl Bahloul
"""Generate TuxMix DeviceProfile modules from oscmix's device_ff*.c tables.

Reads the C files as plain text, never builds or runs them. From the
repository root, with clones of upstream oscmix and of the charlysound fork:

  python3 -I tools/profiles/gen_oscmix_profiles.py \
      <oscmix> <oscmix-ufxplus-linux> tuxmix-core/src/profiles
  rustfmt --edition 2021 tuxmix-core/src/profiles/fireface_*.rs

Upstream oscmix (https://github.com/michaelforney/oscmix) has tables for the
802 and the UCX II; the fork (https://github.com/charlysound/oscmix-ufxplus-linux)
adds the UCX, UFX, UFX II, UFX III and UFX+. Upstream is preferred where its
table is complete.
"""
import re, sys, os

up, fork, out = sys.argv[1:4]

MODELS = [
    # id, file, model_name, card_substring, origin, usb ids, main_pair_label, notes
    ("ucx", "device_ffucx.c", "Fireface UCX", "Fireface UCX", "fork",
     [("vendor", 0x2a39, 0x3fc9), ("vendor", 0x0424, 0x3fc9), ("cc", 0x2a39, 0x3fb9)]),
    ("ucx2", "device_ffucxii.c", "Fireface UCX II", "Fireface UCX II", "upstream",
     [("vendor", 0x2a39, 0x3f82), ("cc", 0x2a39, 0x3fd9)]),
    ("ff802", "device_ff802.c", "Fireface 802", "Fireface 802", "fork", []),
    ("ufx", "device_ffufx.c", "Fireface UFX", "Fireface UFX", "fork", []),
    ("ufx2", "device_ffufxii.c", "Fireface UFX II", "Fireface UFX II", "fork", []),
    ("ufx3", "device_ffufxiii.c", "Fireface UFX III", "Fireface UFX III", "fork", []),
    ("ufxplus", "device_ffufxp.c", "Fireface UFX+", "Fireface UFX+", "fork", []),
]

def strip_comments(s):
    s = re.sub(r"/\*.*?\*/", "", s, flags=re.S)
    return re.sub(r"//[^\n]*", "", s)

def parse(path):
    raw = open(path).read()
    src = strip_comments(raw)
    arrays = {m.group(1): re.findall(r'"([^"]*)"', m.group(2))
              for m in re.finditer(r'static const char \*const (\w+)\[\]\s*=\s*\{(.*?)\};', src, re.S)}
    def table(name):
        m = re.search(r'static const struct channelinfo %s\[\]\s*=\s*\{(.*?)\n\};' % name, src, re.S)
        body = m.group(1)
        entries, depth, cur = [], 0, ""
        for ch in body:
            if ch == "{":
                depth += 1
                if depth == 1:
                    cur = ""
                    continue
            if ch == "}":
                depth -= 1
                if depth == 0:
                    entries.append(cur)
                    continue
            if depth >= 1:
                cur += ch
        res = []
        for e in entries:
            name = re.match(r'\s*"([^"]*)"', e).group(1)
            rest = e[e.index('"', e.index('"') + 1) + 1:]
            flags = set(re.findall(r'(INPUT_HAS_\w+|OUTPUT_HAS_\w+)', rest))
            g = re.search(r'\.gain\s*=\s*\{([^}]*)', rest)
            gain = [int(x) for x in g.group(1).split(",")] if g else None
            r = re.search(r'\.reflevel\s*=\s*\{\s*(\w+)', rest)
            ref = arrays[r.group(1)] if r else []
            res.append(dict(name=name, flags=flags, gain=gain, ref=ref))
        return res
    dev = re.search(r'const struct device \w+\s*=\s*\{(.*?)\n\};', src, re.S).group(1)
    refresh = re.search(r'\.refresh\s*=\s*(0x[0-9A-Fa-f]+)', dev)
    refresh_raw = re.search(r'\.refresh\s*=\s*0x[0-9A-Fa-f]+[^\n]*', raw)
    flags = set(re.findall(r'DEVICE_\w+', re.search(r'\.flags\s*=\s*([^,]*),', dev).group(1)))
    return table("inputs"), table("outputs"), (refresh.group(1) if refresh else None), flags, (refresh_raw.group(0) if refresh_raw else "")

def short(name):
    n = name
    m = re.match(r'^(?:Mic/Line|Mic/Inst|Inst/Line|Mic|Analog|AN)\s+(\d+)$', n)
    if m: return "AN" + m.group(1)
    m = re.match(r'^(?:PH|Phones)\s+(\d+)$', n)
    if m: return "PH" + m.group(1)
    m = re.match(r'^(ADAT|AS|MADI)\s+(\d+)$', n)
    if m: return m.group(1) + m.group(2)
    return n  # "SPDIF L", "AES R"

def ctype(e):
    n, f = e["name"], e["flags"]
    if n.startswith("SPDIF"): return "SPDIF"
    if n.startswith("AES"): return "AES"
    if n.startswith("ADAT") or n.startswith("AS "): return "ADAT"
    if n.startswith("MADI"): return "MADI"
    if "INPUT_HAS_48V" in f or n.startswith("Mic"): return "Mic"
    if "INPUT_HAS_HIZ" in f or n.startswith("Inst"): return "Instrument"
    return "Line"

def rs_str_list(xs):
    return "&[" + ", ".join('"%s"' % x for x in xs) + "]"

def caps(e):
    f = e["flags"]
    parts = []
    if "INPUT_HAS_GAIN" in f and e["gain"]:
        g = e["gain"]
        parts.append("gain: Some(GainRange::db(%d, %d))" % (g[0] // e["scale"], g[1] // e["scale"]))
    if "INPUT_HAS_48V" in f: parts.append("phantom: true")
    if "INPUT_HAS_HIZ" in f: parts.append("hi_z: true")
    if "INPUT_HAS_AUTOSET" in f: parts.append("autoset: true")
    if "INPUT_HAS_REFLEVEL" in f and e["ref"]: parts.append("ref_levels: " + rs_str_list(e["ref"]))
    # oscmix maps the EQ registers (low cut, 3 bands) in every channel's
    # register block, so every input of these units has an EQ strip.
    parts.append("eq: true")
    return "InputCaps { " + ", ".join(parts) + ", ..InputCaps::NONE }"

def pair_label(l, r):
    if l.endswith(" L") and r.endswith(" R") and l[:-2] == r[:-2]:
        return l[:-2]
    rn = re.sub(r'^\D*', '', r)
    return "%s/%s" % (l, rn) if rn else "%s/%s" % (l, r)

UPSTREAM_URL = "https://github.com/michaelforney/oscmix"
FORK_URL = "https://github.com/charlysound/oscmix-ufxplus-linux"

for mid, fname, model, card, origin, usb in MODELS:
    path = os.path.join(up if origin == "upstream" else fork, fname)
    ins, outs, refresh, dflags, refresh_line = parse(path)
    # Upstream oscmix stores preamp ranges in tenths of a dB (it sends
    # `val * 10` to the unit); the fork stores whole dB.
    for e in ins:
        e["scale"] = 10 if origin == "upstream" else 1
    no_range = [short(e["name"]) for e in ins if "INPUT_HAS_GAIN" in e["flags"] and not e["gain"]]
    refresh_note = None
    if refresh is None:
        # Upstream's 802 table has no refresh value; the fork's has one.
        _, _, refresh, _, _ = parse(os.path.join(fork, fname))
        refresh_note = "upstream oscmix gives no refresh value for this unit; %s is the fork's" % refresh
    assert len(outs) % 2 == 0
    lines = []
    A = lines.append
    A("// SPDX-License-Identifier: MIT AND ISC")
    A("// Copyright (c) 2026 Ismaïl Bahloul")
    A("// Channel table derived from oscmix `%s`:" % fname)
    A("// Copyright (c) 2023-2024 Michael Forney, ISC license (see")
    A("// `LICENSES/oscmix-ISC.txt`).")
    if origin == "fork":
        A("// Taken from the charlysound/oscmix-ufxplus-linux fork of oscmix.")
    A("")
    A("//! %s channel topology, for the simulation only." % model)
    A("//!")
    A("//! No backend drives a %s yet: this profile lets `--mock` show the" % model)
    A("//! unit's layout. Generated from oscmix's table for the unit (%s)," % ("upstream" if origin == "upstream" else ("the charlysound fork: upstream's table for this unit gives no gain or reference-level ranges" if mid == "ff802" else "the charlysound fork; upstream oscmix has no table for this unit")))
    A("//! by `tools/profiles/gen_oscmix_profiles.py`; edit the generator, not")
    A("//! this file. Not checked on a unit.")
    if refresh_line and "//" in refresh_line:
        A("//!")
        A("//! The refresh value is not settled in the source: `%s`." % refresh_line.strip().rstrip(","))
    if no_range:
        A("//!")
        A("//! The source marks %s as having a preamp gain but gives no range," % ", ".join(no_range))
        A("//! so the profile leaves it out.")
    if refresh_note:
        A("//!")
        A("//! Refresh: " + refresh_note + ".")
    A("")
    A("use crate::channel::ChannelType;")
    imports = ["ControlProtocol", "DeviceFeatures", "DeviceProfile", "InputSpec", "OutputPairSpec", "Support"]
    body_has_caps = any(caps(e) for e in ins)
    if body_has_caps: imports += ["InputCaps"]
    if any("INPUT_HAS_GAIN" in e["flags"] and e["gain"] for e in ins): imports += ["GainRange"]
    if usb: imports += ["UsbIdentity"]
    A("use crate::profile::{%s};" % ", ".join(sorted(imports)))
    A("")
    A("pub const PROFILE: DeviceProfile = DeviceProfile {")
    A('    id: "%s",' % mid)
    A('    card_substring: "%s",' % card)
    A('    model_name: "%s",' % model)
    A("    inputs: &[")
    for e in ins:
        c = caps(e)
        if c:
            A('        InputSpec::new("%s", ChannelType::%s, %s),' % (short(e["name"]), ctype(e), c))
        else:
            A('        InputSpec::plain("%s", ChannelType::%s),' % (short(e["name"]), ctype(e)))
    A("    ],")
    A("    outputs: &[")
    main = None
    for i in range(0, len(outs), 2):
        l, r = outs[i], outs[i + 1]
        ln, rn = short(l["name"]), short(r["name"])
        lab = pair_label(ln, rn)
        if main is None and ln.startswith("PH"):
            main = i // 2
        if l["ref"]:
            A('        OutputPairSpec::with_ref_levels("%s", "%s", "%s", %s),' % (ln, rn, lab, rs_str_list(l["ref"])))
        else:
            A('        OutputPairSpec::new("%s", "%s", "%s"),' % (ln, rn, lab))
    A("    ],")
    A("    // TotalMix's Main is not documented in the source table; the first")
    A("    // pair stands in for it.")
    A("    main_pair: Some(0),")
    if usb:
        A("    usb: &[")
        for mode, v, p in usb:
            fn = "vendor_mode" if mode == "vendor" else "class_compliant"
            A("        UsbIdentity::%s(0x%04x, 0x%04x)," % (fn, v, p))
        A("    ],")
    else:
        A("    // USB identities not collected yet.")
        A("    usb: &[],")
    A("    protocol: ControlProtocol::Latter { refresh: %s }," % refresh)
    A("    features: DeviceFeatures {")
    A("        durec: %s," % str("DEVICE_HAS_DUREC" in dflags).lower())
    A("        room_eq: %s," % str("DEVICE_HAS_ROOMEQ" in dflags).lower())
    A("        mix_volume_only: %s," % str("DEVICE_MIX_VOLONLY" in dflags).lower())
    A("    },")
    A("    support: Support::SimulationOnly,")
    A('    source: "oscmix %s (%s)",' % (fname, UPSTREAM_URL if origin == "upstream" else FORK_URL))
    A("};")
    open(os.path.join(out, "fireface_%s.rs" % mid), "w").write("\n".join(lines) + "\n")
    print(mid, len(ins), len(outs), refresh, sorted(dflags))
