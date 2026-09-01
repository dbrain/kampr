#!/usr/bin/env python3
"""Builds client/shared/src/commonMain/composeResources/font/terminalmono_*.ttf.

    pip install fonttools
    python3 tools/terminalmono.py --verify      # rebuild in memory, diff against what ships
    python3 tools/terminalmono.py --write       # rebuild the four faces in place

The face is JetBrains Mono NL 2.304 with the symbols it does not carry added, because a browser
has no system font behind Skia and a FontFamily of loaded fonts resolves to exactly one typeface:
a codepoint the face lacks is tofu and nothing else can supply it (#66, #270).

Two ways in, and the choice matters:

  ALIAS  points a new cmap entry at a glyph JetBrains Mono already draws. Used where the codepoint
         is a typographic variant of one — U+2011 is a hyphen, U+2003 is a space — and, more
         importantly, where the glyph belongs to the box-drawing lattice. JetBrains' box glyphs
         deliberately overflow the 600 cell (U+2514 spans x 250..620) so that neighbouring cells
         join; a cut-in centred in 560 lands at x 103..496 and the tree visibly breaks apart.
         Aliasing also keeps the weight of the face, which a cut-in cannot: the bold face gets a
         bold elbow.

  CUT_IN copies an outline out of a Noto face (all OFL 1.1, all 1000 upem), scaled by
         min(1, 560/ink_width, 900/ink_height) about the baseline origin and centred in the 600
         advance. That rule was recovered by fitting the 834 glyphs already in the shipped face
         and reproduces every one of them (#271).

Nothing here may widen a cell or grow a line: every glyph added is given the 600 advance, and the
vertical metrics are JetBrains Mono's untouched. `--verify` asserts both.
"""

import argparse
import pathlib
import sys

from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont

ADVANCE = 600
INK_WIDTH = 560
INK_HEIGHT = 900

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "client/shared/src/commonMain/composeResources/font"

FACES = [
    ("regular", "Regular"),
    ("bold", "Bold"),
    ("italic", "Italic"),
    ("bolditalic", "BoldItalic"),
]

BASE = "/usr/share/fonts/TTF/JetBrainsMonoNL-{}.ttf"

# Ordered: the first donor carrying a codepoint wins. Noto Sans Symbols 2 is first because it is
# the donor every glyph in the original build came from.
DONORS = [
    "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
    "/usr/share/fonts/noto/NotoSansSymbols-Regular.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/noto/NotoSansMath-Regular.ttf",
]

# codepoint -> the codepoint whose glyph it borrows.
ALIAS = {
    0x23BF: 0x2514,  # ⎿ is drawn as └: it is the elbow of an agent's tool-result tree and has to
    0x23AF: 0x2500,  # ⎯ as ─, for the same reason — both sit in the box lattice (#271)
    0x2011: 0x002D,  # non-breaking hyphen
    0x202F: 0x0020,  # narrow no-break space
    0x2003: 0x0020,  # em space — one cell, like every other cell
    0x2705: 0x2713,  # ✅ as ✓ and ❌ as ✗: the emoji-presentation twins of a check and a
    0x274C: 0x2717,  # ballot X, and no monochrome donor on any build machine carries either —
                     # Noto Sans Symbols 2 stops short of the emoji dingbats and Noto Color Emoji
                     # is CBDT bitmaps with no outline to cut. Aliasing is also the only path that
                     # keeps the weight, so a bold pane gets a bold tick
}

# Everything an agent harness draws that JetBrains Mono NL does not carry and that is not in the
# lattice. Measured: #270.
CUT_IN = [
    0x21B3, 0x21B9, 0x21C4, 0x21F1, 0x2153, 0x2154, 0x207F, 0x2315, 0x2460, 0x2461, 0x2462,
    0x2463, 0x2464, 0x2465, 0x2691, 0x2699, 0x27FA, 0x29C9, 0x1F5BC, 0x1F6E1,
]


def ink_bounds(glyph_set, name):
    pen = BoundsPen(glyph_set)
    glyph_set[name].draw(pen)
    return pen.bounds


def scale_for(bounds):
    width = bounds[2] - bounds[0]
    height = bounds[3] - bounds[1]
    limits = [1.0]
    if width > 0:
        limits.append(INK_WIDTH / width)
    if height > 0:
        limits.append(INK_HEIGHT / height)
    return min(limits)


def outline(donor, codepoint):
    donor_name = donor.getBestCmap()[codepoint]
    donor_set = donor.getGlyphSet()
    bounds = ink_bounds(donor_set, donor_name)
    if bounds is None:
        return TTGlyphPen(None).glyph()  # U+2800 and friends are blank by definition
    scale = scale_for(bounds)
    dx = (ADVANCE - (bounds[2] - bounds[0]) * scale) / 2 - bounds[0] * scale

    flat = DecomposingRecordingPen(donor_set)
    donor_set[donor_name].draw(flat)
    pen = TTGlyphPen(None)
    flat.replay(TransformPen(pen, (scale, 0, 0, scale, dx, 0)))
    return pen.glyph()


def add(font, codepoint, name):
    for table in font["cmap"].tables:
        if not table.isUnicode():
            continue
        if codepoint > 0xFFFF and table.format == 4:
            continue  # format 4 is BMP only; the format 12 subtables carry the astral planes
        table.cmap.setdefault(codepoint, name)


def build(face, base_style, donors, extra_aliases, extra_cut_ins):
    font = TTFont(BASE.format(base_style))
    glyf, hmtx = font["glyf"], font["hmtx"]
    glyf.glyphs  # force the table to decompile before the glyph order moves under it
    cmap = font.getBestCmap()
    order = list(font.getGlyphOrder())

    for codepoint, source in extra_aliases.items():
        if codepoint in cmap:
            continue
        if source not in cmap:
            raise SystemExit(f"U+{codepoint:04X}: alias source U+{source:04X} is not in the base face")
        add(font, codepoint, cmap[source])

    fresh = {}
    for codepoint in extra_cut_ins:
        if codepoint in cmap:
            continue
        donor = next((d for d in donors if codepoint in d.getBestCmap()), None)
        if donor is None:
            raise SystemExit(f"U+{codepoint:04X}: no donor carries it")
        name = f"uni{codepoint:04X}"
        while name in order or name in fresh:
            name += ".cutin"
        fresh[name] = (codepoint, outline(donor, codepoint))

    if fresh:
        font.setGlyphOrder(order + list(fresh))
        font["maxp"].numGlyphs = len(order) + len(fresh)
        for name, (codepoint, glyph) in fresh.items():
            glyf.glyphs[name] = glyph
            glyph.recalcBounds(glyf)
            hmtx.metrics[name] = (ADVANCE, glyph.xMin if glyph.numberOfContours else 0)
            add(font, codepoint, name)

    return font


def existing_cut_ins(shipped, base):
    return sorted(set(shipped.getBestCmap()) - set(base.getBestCmap()))


def verify():
    donors = [TTFont(p) for p in DONORS]
    bad = 0
    for face, style in FACES:
        shipped = TTFont(OUT / f"terminalmono_{face}.ttf")
        base = TTFont(BASE.format(style))
        want = existing_cut_ins(shipped, base)
        rebuilt = build(face, style, donors, ALIAS, [c for c in want if c not in ALIAS])
        rebuilt_map, shipped_map = rebuilt.getBestCmap(), shipped.getBestCmap()
        rset, sset = rebuilt.getGlyphSet(), shipped.getGlyphSet()
        missing = [c for c in want if c not in rebuilt_map]
        moved = []
        for codepoint in want:
            if codepoint in missing:
                continue
            a = ink_bounds(rset, rebuilt_map[codepoint])
            b = ink_bounds(sset, shipped_map[codepoint])
            if a is None and b is None:
                continue
            if a is None or b is None or max(abs(x - y) for x, y in zip(a, b)) > 1.5:
                moved.append((codepoint, a, b))
        print(f"{face:12s} cut-ins {len(want):4d}  unsourced {len(missing):3d}  outline mismatch {len(moved):3d}")
        for codepoint, a, b in moved[:6]:
            print(f"    U+{codepoint:04X} rebuilt={a} shipped={b}")
        bad += len(missing) + len(moved)
    return bad


def widths(font):
    cmap = font.getBestCmap()
    return {cp: font["hmtx"][name][0] for cp, name in cmap.items()}


def write():
    donors = [TTFont(p) for p in DONORS]
    for face, style in FACES:
        shipped = TTFont(OUT / f"terminalmono_{face}.ttf")
        base = TTFont(BASE.format(style))
        want = existing_cut_ins(shipped, base)
        cut_ins = sorted(set(c for c in want if c not in ALIAS) | set(CUT_IN))
        font = build(face, style, donors, ALIAS, cut_ins)

        cmap = font.getBestCmap()
        for codepoint in list(ALIAS) + CUT_IN:
            if codepoint not in cmap:
                raise SystemExit(f"{face}: U+{codepoint:04X} did not make it in")

        # Nothing added may be anything but one cell wide, and nothing inherited may move at all.
        built, inherited = widths(font), widths(base)
        for codepoint in set(cmap) - set(base.getBestCmap()):
            if built[codepoint] != ADVANCE:
                raise SystemExit(f"{face}: U+{codepoint:04X} has advance {built[codepoint]}, not {ADVANCE}")
        for codepoint, advance in inherited.items():
            if built.get(codepoint) != advance:
                raise SystemExit(f"{face}: U+{codepoint:04X} advance moved {advance} -> {built.get(codepoint)}")
        for table, ours, theirs in (
            ("hhea", (font["hhea"].ascent, font["hhea"].descent, font["hhea"].lineGap),
             (base["hhea"].ascent, base["hhea"].descent, base["hhea"].lineGap)),
            ("OS/2", (font["OS/2"].sTypoAscender, font["OS/2"].sTypoDescender,
                      font["OS/2"].usWinAscent, font["OS/2"].usWinDescent),
             (base["OS/2"].sTypoAscender, base["OS/2"].sTypoDescender,
              base["OS/2"].usWinAscent, base["OS/2"].usWinDescent)),
        ):
            if ours != theirs:
                raise SystemExit(f"{face}: {table} vertical metrics drifted: {ours} != {theirs}")

        path = OUT / f"terminalmono_{face}.ttf"
        font.save(path)
        print(f"{face:12s} {len(cmap):5d} codepoints -> {path.relative_to(ROOT)}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    if args.verify:
        sys.exit(1 if verify() else 0)
    elif args.write:
        write()
    else:
        parser.print_help()
