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

It also re-shapes anything that only stays inside the tile *because* of a
clip-path, and then drops the clip. That one is not cosmetic: Qt's SVG
engine, the one KDE's icon loader rasterises with, ignores `clip-path` on a
group -- reproduced with a ten-line file, where Inkscape clips a 100x100
rect to 60x60+20+20 and Qt paints all 100x100. The tile's own light (a
vignette rect and a sheen ellipse, both far larger than the tile) was the
one thing relying on it, so Qt painted both across the whole canvas: a grey
haze around the icon on the desktop, invisible in every icon editor. Their
gradients are userSpaceOnUse, so the same paint on a tile-shaped rounded
rect is the same picture inside the tile with nothing left to clip.

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
the cleaning itself is the standard library. If PyQt6 is installed, the
script also renders the result through Qt and fails loudly if Qt's ink
extends past the tile, which is the check that catches the clipping bug.
"""

import os
import re
import sys
import tempfile
import xml.etree.ElementTree as ET

SVG = "http://www.w3.org/2000/svg"


def local(tag):
    return tag.split("}", 1)[-1]


def fix_clip_reliance(root):
    """Re-shape what a clip-path is holding in, then drop the clip.

    See the module docstring: Qt ignores `clip-path` on a group, so anything
    that stays on the tile only because it is clipped to it leaks across the
    whole canvas on a KDE desktop. Only the leaking elements' *shape* changes
    (a rect and an ellipse both become the clip's rounded rect, keeping their
    paint), which is the same picture as long as the paint is a
    userSpaceOnUse gradient. Anything the tool cannot account for is left
    alone, clip and all, and said so out loud.

    Returns the tile rect (x, y, width, height) if it found one.
    """
    tile = None

    def rect_of(el):
        if el is None or local(el.tag) != "rect":
            return None
        return tuple(float(el.get(name, 0)) for name in ("x", "y", "width", "height"))

    def bbox_of(el):
        kind = local(el.tag)
        if kind == "rect":
            x, y, w, h = rect_of(el)
            return x, y, w, h
        if kind == "ellipse":
            cx, cy = float(el.get("cx", 0)), float(el.get("cy", 0))
            rx, ry = float(el.get("rx", 0)), float(el.get("ry", 0))
            return cx - rx, cy - ry, 2 * rx, 2 * ry
        return None

    def fits(box, frame):
        x, y, w, h = box
        fx, fy, fw, fh = frame
        return x >= fx and y >= fy and x + w <= fx + fw and y + h <= fy + fh

    defs = next(el for el in root if local(el.tag) == "defs")
    by_id = {el.get("id"): el for el in defs.iter() if el.get("id")}
    for group in list(root):
        match = re.fullmatch(r"url\(#([^)]+)\)", group.get("clip-path") or "")
        if not match:
            continue
        clip = by_id.get(match.group(1))
        shapes = list(clip) if clip is not None and local(clip.tag) == "clipPath" else []
        if len(shapes) != 1 or rect_of(shapes[0]) is None:
            print(f"kept clip-path #{match.group(1)}: its shape is not a single rect")
            continue
        frame = tile = rect_of(shapes[0])

        for child in list(group):
            box = bbox_of(child)
            if box is None or fits(box, frame):
                continue
            paint = {
                name: value
                for name, value in child.attrib.items()
                if name not in ("x", "y", "width", "height", "rx", "cx", "cy", "ry")
            }
            shape = {n: v for n, v in shapes[0].attrib.items() if n != "id"}
            group.insert(list(group).index(child), ET.Element(shapes[0].tag, {**paint, **shape}))
            group.remove(child)
            print(
                f"re-shaped a <{local(child.tag)} fill={paint.get('fill')}> onto the tile: "
                "it was only inside it because of the clip"
            )

        unknown = [child for child in group if bbox_of(child) is None]
        outside = [child for child in group if (box := bbox_of(child)) and not fits(box, frame)]
        if unknown or outside:
            print(
                f"kept clip-path #{match.group(1)}: {len(unknown)} child(ren) it cannot check, "
                f"{len(outside)} still outside"
            )
            continue
        del group.attrib["clip-path"]
        print(f"dropped clip-path #{match.group(1)}: every child now fits the tile by shape")

    return tile


def qt_ink_check(path, view, tile, scale=512):
    """Render through Qt and make sure its ink stops at the tile.

    Qt's engine is what KDE rasterises with, and the only one that shows the
    clip bug: with it, the icon's ink covered the whole canvas instead of the
    tile. Skipped when PyQt6 is not installed.
    """
    try:
        os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
        from PyQt6.QtCore import QRectF
        from PyQt6.QtGui import QGuiApplication, QImage, QPainter
        from PyQt6.QtSvg import QSvgRenderer
    except ImportError:
        print("PyQt6 is not installed: skipped the Qt ink check")
        return

    app = QGuiApplication.instance() or QGuiApplication([])
    renderer = QSvgRenderer(path)
    image = QImage(scale, scale, QImage.Format.Format_ARGB32_Premultiplied)
    image.fill(0)
    painter = QPainter(image)
    renderer.render(painter, QRectF(0, 0, scale, scale))
    painter.end()
    # In the temp directory, not next to the file being written: a check
    # render is not an artefact the repo should ever carry.
    shot = os.path.join(tempfile.gettempdir(), "tuxmix-qt-check.png")
    image.save(shot)

    # `pixel` returns 0xAARRGGBB for this format, so the top byte is alpha.
    lit = [(x, y) for y in range(scale) for x in range(scale) if image.pixel(x, y) >> 24]
    if not lit:
        print("Qt drew nothing at all")
        raise SystemExit(1)
    xs = [x for x, _ in lit]
    ys = [y for _, y in lit]
    min_x, max_x, min_y, max_y = min(xs), max(xs), min(ys), max(ys)

    box = f"{max_x - min_x + 1}x{max_y - min_y + 1}+{min_x}+{min_y}"
    if tile is None:
        # Nothing was clipped, so there is no tile box to compare against.
        # The design insets the tile in its viewBox (3% each side), so ink
        # that reaches the canvas edge is ink that escaped something, which
        # is what the clip bug looked like; ink merely near the edge is
        # normal. Deliberately a weak check, and said out loud as one.
        margin = scale / 100
        if min_x < margin or min_y < margin or max_x > scale - margin or max_y > scale - margin:
            print(f"Qt ink at {scale}px: {box} reaches the canvas edge: something paints "
                  f"outside the icon ({shot})")
            raise SystemExit(1)
        print(f"Qt ink at {scale}px: {box}, inset from the edges (no clip in the file to compare "
              f"against): OK ({shot})")
        return
    tx, ty, tw, th = (value * scale / view for value in tile)
    # A pixel of slack each way: the tile's own edge is antialiased.
    inside = (
        tx - 1 <= min_x
        and ty - 1 <= min_y
        and max_x <= tx + tw + 1
        and max_y <= ty + th + 1
    )
    expected = f"{tw:.0f}x{th:.0f}+{tx:.0f}+{ty:.0f}"
    if inside:
        print(f"Qt ink at {scale}px: {box}, the tile ({expected}) and nothing else: OK ({shot})")
    else:
        print(f"Qt ink at {scale}px: {box}, expected the tile {expected}: Qt paints outside the icon ({shot})")
        raise SystemExit(1)


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
    view = float((root.get("viewBox") or "0 0 1024 1024").split()[2])

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
    tile = fix_clip_reliance(root)

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

    qt_ink_check(dst, view, tile)
    return 0


if __name__ == "__main__":
    sys.exit(main())
