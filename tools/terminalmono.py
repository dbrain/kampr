#!/usr/bin/env python3
"""Builds client/shared/src/commonMain/composeResources/font/terminalmono_*.ttf.

    pip install fonttools
    python3 tools/terminalmono.py --verify      # rebuild in memory, diff against what ships
    python3 tools/terminalmono.py --write       # rebuild the four faces in place
    python3 tools/terminalmono.py --gaps        # regenerate GlyphGaps.kt from the shipped faces

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

  CUT_IN copies an outline out of a Noto face (all OFL 1.1), scaled by
         min(1, 560/ink_width, 900/ink_height) about the baseline origin and centred in the 600
         advance. That rule was recovered by fitting the 834 glyphs already in the shipped face
         and reproduces every one of them (#271).

Nothing here may widen a cell or grow a line: every glyph added is given the 600 advance, and the
vertical metrics are JetBrains Mono's untouched. `--verify` asserts both.

**Emoji are taken whole rather than listed** (#417). Every named codepoint before them came off a corpus
census, and that method keeps losing to the next symbol nobody had seen yet — #404 closed a gap
that #270 had filtered out, and the report after it was a headphone an artifact chose as its own
icon. There is no list to keep up with: a harness prints whatever a person or a tool picked. So the
monochrome Noto Emoji face is cut in entire, and the class stops being a report.

That donor is **vendored** in `tools/donors/`, unlike the four above: no distribution packages a
monochrome Noto Emoji — `noto-fonts-emoji` is the CBDT colour face, which has no outline to cut —
and the file here is `ofl/notoemoji/NotoEmoji[wght].ttf` from google/fonts instantiated at
`wght=400`. OFL 1.1, licence beside it.
"""

import argparse
import pathlib
import sys
import unicodedata

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

EMOJI = ROOT / "tools/donors/NotoEmoji-Regular.ttf"

# Ordered: the first donor carrying a codepoint wins. Noto Sans Symbols 2 is first because it is
# the donor every glyph in the original build came from, and the emoji face is **last** so that a
# codepoint one of the symbol faces already draws keeps the line-art glyph it has always had —
# U+2699 is a gear in Symbols and a filled emoji cog in Noto Emoji, and the pane wants the gear.
DONORS = [
    "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
    "/usr/share/fonts/noto/NotoSansSymbols-Regular.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/noto/NotoSansMath-Regular.ttf",
    str(EMOJI),
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


def emoji_cut_ins(base):
    """Every codepoint the vendored emoji face draws that the base family does not.

    Whole rather than listed, and not filtered by anything: the reason this class keeps coming back
    as a report is that every filter applied to it has been a guess about what somebody would print.
    """
    return sorted(set(TTFont(EMOJI).getBestCmap()) - set(base.getBestCmap()))


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


# ---------------------------------------------------------------------------------------------
# The other half: the UI faces are stock and cannot be rebuilt, so prose gets its symbols routed.
# ---------------------------------------------------------------------------------------------

GAPS_OUT = ROOT / "client/shared/src/commonMain/kotlin/dev/kampr/shared/theme/GlyphGaps.kt"

# The `FamilyId` each shipped file belongs to, which is the key a theme picks a face by.
UI_FAMILIES = {
    "manrope": "Manrope",
    "ibmplexmono": "IbmPlexMono",
    "jetbrainsmono": "JetBrainsMono",
    "instrumentsans": "InstrumentSans",
    "archivo": "Archivo",
}


def coverage(prefix):
    """Every codepoint any weight of one family draws.

    The union across weights, not one face: a `FontFamily` picks a face by weight and a codepoint
    only one weight carries is drawn *somewhere*, so treating it as absent would route a glyph that
    already renders.
    """
    covered = set()
    for path in sorted(OUT.glob(f"{prefix}_*.ttf")):
        covered |= set(TTFont(path).getBestCmap())
    return covered


def as_ranges(codepoints):
    ranges, start, prev = [], None, None
    for cp in sorted(codepoints):
        if start is None:
            start = prev = cp
        elif cp == prev + 1:
            prev = cp
        else:
            ranges.append((start, prev))
            start = prev = cp
    if start is not None:
        ranges.append((start, prev))
    return ranges


# Only a standalone symbol, punctuation mark or symbolic numeral may be routed.
#
# **A combining mark must stay with its base and a format character must stay invisible.** A mark
# drawn from a different face than the letter it sits on is a mark that lands in the wrong place,
# and a zero-width joiner given a family of its own is a run split down the middle of an emoji
# sequence — U+200D, U+FE0F and the tag characters at U+E0030 are exactly what holds a flag or a
# family emoji together. Letters are out of scope for the same reason they are out of scope for the
# face itself: a script this cannot draw goes down the fallback path #214 built, not through here.
ROUTABLE = ("Sm", "Sc", "Sk", "So", "Pd", "Ps", "Pe", "Pi", "Pf", "Po", "Pc", "No")


def routable(codepoint):
    try:
        return unicodedata.category(chr(codepoint)) in ROUTABLE
    except ValueError:
        return False


def gap_table():
    """Per family: what the terminal face draws, that family does not, and prose may re-aim.

    One direction only. A codepoint the UI face has and the terminal face does not — 14 to 40 of
    them per family, currency signs almost entirely — is left exactly where it is: this table may
    turn tofu into a glyph and may never move a glyph that already draws.
    """
    terminal = coverage("terminalmono")
    return {
        name: as_ranges(cp for cp in terminal - coverage(prefix) if routable(cp))
        for prefix, name in sorted(UI_FAMILIES.items())
    }


def gaps_source():
    table = gap_table()
    lines = [
        "package dev.kampr.shared.theme",
        "",
        "// GENERATED by `python3 tools/terminalmono.py --gaps`. Do not edit by hand.",
        "//",
        "// Per family, the codepoints the terminal face draws and that family does not — so a piece",
        "// of prose can have exactly those routed to the terminal face and nothing else. It is",
        "// generated rather than written because it is a fact about the shipped .ttf files and",
        "// changes whenever they do; `GlyphGapsTest` regenerates it and fails on any drift.",
        "//",
        "// Why routing at all: a `FontFamily` of loaded fonts draws everything from its first font",
        "// and a second face in it supplies nothing (#416), so the only way a proportional face can",
        "// show a symbol it does not carry is for the text to name a different family over that",
        "// range. Flat `[first, last]` pairs, ascending and non-adjacent, for a binary search.",
        "",
    ]
    for name, ranges in table.items():
        flat = ", ".join(f"0x{a:X}, 0x{b:X}" for a, b in ranges)
        lines.append(f"internal val {name.upper()}_GAPS: IntArray = intArrayOf({flat})")
        lines.append("")
    lines.append("internal fun gapsFor(id: FamilyId): IntArray = when (id) {")
    for name in table:
        lines.append(f"    FamilyId.{name} -> {name.upper()}_GAPS")
    lines.append("}")
    lines.append("")
    return "\n".join(lines)


def gaps():
    GAPS_OUT.write_text(gaps_source())
    table = gap_table()
    for name, ranges in table.items():
        print(f"{name:16s} {sum(b - a + 1 for a, b in ranges):5d} codepoints in {len(ranges):4d} ranges")
    print(f"-> {GAPS_OUT.relative_to(ROOT)}")


def widths(font):
    cmap = font.getBestCmap()
    return {cp: font["hmtx"][name][0] for cp, name in cmap.items()}


def write():
    donors = [TTFont(p) for p in DONORS]
    for face, style in FACES:
        shipped = TTFont(OUT / f"terminalmono_{face}.ttf")
        base = TTFont(BASE.format(style))
        want = existing_cut_ins(shipped, base)
        emoji = emoji_cut_ins(base)
        cut_ins = sorted(set(c for c in want if c not in ALIAS) | set(CUT_IN) | set(emoji))
        font = build(face, style, donors, ALIAS, cut_ins)

        cmap = font.getBestCmap()
        for codepoint in list(ALIAS) + CUT_IN + emoji:
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
    parser.add_argument("--gaps", action="store_true")
    args = parser.parse_args()
    if args.verify:
        sys.exit(1 if verify() else 0)
    elif args.write:
        write()
        gaps()
    elif args.gaps:
        gaps()
    else:
        parser.print_help()
