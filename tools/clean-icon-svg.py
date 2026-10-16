#!/usr/bin/env python3
"""Clean an Inkscape export of the product icon down to the repo's SVG.

    python3 tools/clean-icon-svg.py SRC [DST]      # DST defaults to assets/tuxmix.svg

What it drops, in order: the sodipodi/inkscape/rdf/cc/dc/c2pa namespaces and
the `namedview` they live on, the `metadata` block (a C2PA provenance
manifest, in base64, which is most of an export's bytes), the `display:none`
layers left behind while drawing, every `style` declaration that is either a
no-op or a disagreement with the attribute it overrides, every `id` nothing
points at, and any `defs` entry left unreferenced. The drawing itself is
never touched -- verify that with the pixel comparison below, where a clean
run must return exactly 0.

The whole icon pipeline, for a new export from the maintainer:

    python3 tools/clean-icon-svg.py ~/Desktop/tuxmix-final.svg assets/tuxmix.svg
    inkscape --export-type=png --export-filename=/tmp/export-256.png \
        --export-width=256 ~/Desktop/tuxmix-final.svg
    inkscape --export-type=png --export-filename=/tmp/clean-256.png \
        --export-width=256 assets/tuxmix.svg
    magick compare -metric AE /tmp/export-256.png /tmp/clean-256.png null:   # must print 0
    inkscape --export-type=png --export-filename=assets/tuxmix-128.png --export-width=128 assets/tuxmix.svg
    inkscape --export-type=png --export-filename=assets/tuxmix-256.png --export-width=256 assets/tuxmix.svg
    cp assets/tuxmix.svg assets/tuxmix-128.png assets/tuxmix-256.png tuxmix-gui/assets/
    cargo test --workspace          # the_packaged_icon_decodes_as_256_rgba pins the PNG

`inkscape` and `magick` are only needed for the verification and the PNGs;
the cleaning itself is the standard library.
"""

import re
import sys
import xml.etree.ElementTree as ET

SVG = "http://www.w3.org/2000/svg"


def main() -> int:
    if len(sys.argv) not in (2, 3):
        print(__doc__.strip().splitlines()[0], file=sys.stderr)
        print("usage: clean-icon-svg.py SRC [DST]", file=sys.stderr)
        return 2
    src, dst = sys.argv[1], (sys.argv[2] if len(sys.argv) == 3 else "assets/tuxmix.svg")

    # Without this, ElementTree invents an `ns0:` prefix for the SVG namespace
    # and writes it on every element.
    ET.register_namespace("", SVG)
    root = ET.fromstring(open(src, encoding="utf-8").read())

    def local(tag):
        return tag.split("}", 1)[-1]

    def clean(el):
        for child in list(el):
            if local(child.tag) in ("namedview", "metadata"):
                el.remove(child)
                continue
            if "display:none" in child.get("style", "").replace(" ", ""):
                el.remove(child)
                continue
            clean(child)

    clean(root)

    # Non-SVG-namespace attributes (inkscape:label and friends) would dangle
    # once their namespace declaration is gone.
    for el in root.iter():
        for name in list(el.attrib):
            if name.startswith("{") and not name.startswith("{" + SVG):
                del el.attrib[name]

    root.attrib.pop("id", None)
    root.attrib.pop("version", None)
    root.set("role", "img")
    root.set("aria-label", "TuxMix icon")
    for el in root:
        if local(el.tag) == "title":
            el.text = "TuxMix icon: the bird and waveform mark on a dark tile"

    # `style` alongside presentation attributes is Inkscape's own doing. Most
    # of what is left here is a no-op (display:inline, mix-blend-mode:normal,
    # opacity:1, or a fill:url() that repeats the element's own fill). The
    # rest is not a duplicate but a disagreement, and the style is the one
    # that wins, so folding each declaration into the attribute it overrides
    # keeps the cascade result and removes the contradiction.
    noop = {"display:inline", "mix-blend-mode:normal", "opacity:1"}
    for el in root.iter():
        style = el.get("style")
        if style is None:
            continue
        folded = {}
        for decl in style.split(";"):
            decl = decl.strip()
            if not decl or decl in noop or decl == f"fill:{el.get('fill')}":
                continue
            prop, _, value = decl.partition(":")
            folded[prop] = value
        del el.attrib["style"]
        for prop, value in folded.items():
            if el.get(prop) != value:
                print(f"folded style into attribute: {local(el.tag)} {prop}={value}")
                el.set(prop, value)

    def referenced():
        used = set()
        for el in root.iter():
            if el is defs:
                continue
            for value in el.attrib.values():
                used.update(re.findall(r"url\(#([^)]+)\)", value))
        return used

    # Anything left in `defs` that nothing points at is a leftover too (a mask
    # or a blur kept alive only by the layers just dropped). Looped because
    # dropping one entry can orphan another. Runs before the id pass so its
    # log can name what it drops.
    defs = next(el for el in root if local(el.tag) == "defs")
    while True:
        orphans = [child for child in defs if child.get("id") not in referenced()]
        if not orphans:
            break
        for orphan in orphans:
            defs.remove(orphan)
        print("dropped orphaned defs:", [o.get("id") for o in orphans])

    # Nothing references these ids either: `url(#...)` targets and nothing else.
    while True:
        used = referenced()
        dropped = []
        for el in root.iter():
            if el is not root and "id" in el.attrib and el.get("id") not in used:
                dropped.append(el.get("id"))
                del el.attrib["id"]
        if not dropped:
            break
        print("dropped unreferenced ids:", len(dropped))

    ET.indent(root, space="  ")
    out = "<?xml version='1.0' encoding='utf-8'?>\n" + ET.tostring(root, encoding="unicode") + "\n"
    open(dst, "w", encoding="utf-8").write(out)
    print(f"wrote {dst}: {len(out)} bytes, {len(out.splitlines())} lines")
    return 0


if __name__ == "__main__":
    sys.exit(main())
