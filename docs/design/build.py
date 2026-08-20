#!/usr/bin/env python3
"""Assemble the Kampr .dc.html artboards from parts/ + the shared theme layer.

Every artboard is themeable: the root carries data-theme, and every colour, font
and radius resolves through a CSS custom property. Flipping the `theme` tweak
re-skins the whole screen, which is the proof that phosphor / warm / brutalist
stay reachable later.
"""
import pathlib, sys

HERE = pathlib.Path(__file__).parent
PARTS = HERE / "parts"

FONTS = ("https://fonts.googleapis.com/css2?"
         "family=Manrope:wght@400;500;600;700;800"
         "&family=IBM+Plex+Mono:wght@400;500;600"
         "&family=JetBrains+Mono:wght@400;500;700"
         "&family=Instrument+Sans:wght@400;500;600"
         "&family=Archivo:wght@500;700;900"
         "&display=swap")

# The Kotlin token layer is the source of truth (client/shared/.../theme/Themes.kt and
# TerminalPalette.kt); these tables are the same values so an artboard and a build agree.
DARK = {
  "soft": """--bg: #0e0f13; --bar: #12141a; --surface: #171a21; --surface-2: #090a0d;
      --raise: #22262f; --line: #1e2129;
      --text: #e8eaf0; --dim: #8d93a3; --mute: #545a68;
      --accent: #6ea8fe; --accent-hi: #9cc4ff; --on-accent: #0e0f13;
      --accent-soft: #16203050;
      --blocked: #ff6b6b; --blocked-bg: #2c1b20; --working: #ffc857;
      --idle: #5b6172; --done: #58d68d;""",
  "phosphor": """--bg: #0a0b0a; --bar: #0c0e0c; --surface: #101210; --surface-2: #070807;
      --raise: #171a17; --line: #1d211d;
      --text: #cdd6cd; --dim: #6b756b; --mute: #4d554d;
      --accent: #f5c542; --accent-hi: #ffd968; --on-accent: #0a0b0a;
      --accent-soft: #16130a;
      --blocked: #ff5f56; --blocked-bg: #150d0c; --working: #f5c542;
      --idle: #4a534a; --done: #57c46b;""",
  "warm": """--bg: #12100e; --bar: #171412; --surface: #1a1714; --surface-2: #0d0b0a;
      --raise: #241f1b; --line: #221e1a;
      --text: #efe7dc; --dim: #8d8175; --mute: #5f574e;
      --accent: #e0a458; --accent-hi: #f0bd7c; --on-accent: #12100e;
      --accent-soft: #211a12;
      --blocked: #e0685a; --blocked-bg: #2a1613; --working: #e0a458;
      --idle: #4a423a; --done: #7fb069;""",
  "brutalist": """--bg: #000000; --bar: #000000; --surface: #000000; --surface-2: #000000;
      --raise: #000000; --line: #ffffff;
      --text: #ffffff; --dim: #8a8a8a; --mute: #5c5c5c;
      --accent: #ff2b1f; --accent-hi: #ff6a60; --on-accent: #000000;
      --accent-soft: #000000;
      --blocked: #ff2b1f; --blocked-bg: #000000; --working: #ffffff;
      --idle: #333333; --done: #ffffff;""",
}

LIGHT = {
  "soft": """--bg: #f7f8fb; --bar: #eef1f7; --surface: #ffffff; --surface-2: #e7ebf2;
      --raise: #e2e7f0; --line: #d8dee9;
      --text: #12151b; --dim: #565d6d; --mute: #6a7183;
      --accent: #2560cc; --accent-hi: #17439a; --on-accent: #ffffff;
      --accent-soft: #a8c4f030;
      --blocked: #b8281f; --blocked-bg: #fbe4e1; --working: #8a5a00;
      --idle: #9aa1b2; --done: #0f6b45;""",
  "phosphor": """--bg: #f3f5ef; --bar: #eaeee4; --surface: #fbfcf8; --surface-2: #e2e7d9;
      --raise: #dde3d3; --line: #cdd5c1;
      --text: #0f150d; --dim: #4d5749; --mute: #68725f;
      --accent: #7a5400; --accent-hi: #5c3f00; --on-accent: #ffffff;
      --accent-soft: #f0e6c8;
      --blocked: #a82015; --blocked-bg: #f8e3e0; --working: #7a5400;
      --idle: #98a291; --done: #1d5e2a;""",
  "warm": """--bg: #faf6ef; --bar: #f3ecdf; --surface: #fffdf8; --surface-2: #efe7d8;
      --raise: #eae0cd; --line: #ded3bf;
      --text: #1c1610; --dim: #635749; --mute: #7b6f5d;
      --accent: #8f5210; --accent-hi: #6e3c06; --on-accent: #fffdf8;
      --accent-soft: #f2e3cb;
      --blocked: #a83521; --blocked-bg: #f8e2da; --working: #8f5210;
      --idle: #aa9c86; --done: #3a6127;""",
  "brutalist": """--bg: #ffffff; --bar: #ffffff; --surface: #ffffff; --surface-2: #ffffff;
      --raise: #ffffff; --line: #000000;
      --text: #000000; --dim: #4a4a4a; --mute: #5e5e5e;
      --accent: #cc0a00; --accent-hi: #960700; --on-accent: #ffffff;
      --accent-soft: #ffffff;
      --blocked: #cc0a00; --blocked-bg: #ffffff; --working: #000000;
      --idle: #b5b5b5; --done: #000000;""",
}

# ADR 0009: the terminal keeps a dark ground under both app grounds, so there is exactly one
# terminal block per theme and it does not appear in LIGHT.
TERM = {
  "soft": ("#0b0d12", "#dde3ee", ["#1b1f27","#f2707a","#5fd68f","#e9c46a","#7fb0ff","#c79be0","#5fd2d2","#c6ccd9",
      "#7c8496","#ff8e96","#86e9ab","#f7d98c","#a6c8ff","#ddb8f0","#8ce7e7","#f2f5fa"]),
  "phosphor": ("#050705", "#c9d5c7", ["#131a13","#ff5f4a","#57c46b","#f5c542","#58a6b8","#d98ba0","#7fe0c8","#c4cfc2",
      "#6f7d6d","#ff8a72","#7fdc8e","#ffd968","#7fc8d8","#eeadbe","#a6f0de","#e9f0e7"]),
  "warm": ("#15110d", "#eee4d5", ["#241f1a","#d9604e","#7fb069","#e0a458","#7a93c4","#c48aa8","#79b8b0","#d8cdbc",
      "#8a7d6d","#f07a64","#97c77f","#f0bd7c","#9ab0da","#dca5c1","#95d0c8","#f6efe3"]),
  "brutalist": ("#000000", "#ffffff", ["#1a1a1a","#ff2b1f","#00e05a","#ffe500","#4d7cff","#ff3ddb","#00e5e5","#e6e6e6",
      "#8a8a8a","#ff6a60","#5cff9e","#fff35c","#8faeff","#ff8ae8","#6bffff","#ffffff"]),
}

SHAPE = {
  "soft": "",
  "phosphor": """
      --font-ui: 'JetBrains Mono', ui-monospace, monospace;
      --font-mono: 'JetBrains Mono', ui-monospace, monospace;
      --r-lg: 2px; --r-md: 2px; --r-sm: 2px;
      --card-border: 1px solid var(--line);
      --label-tt: uppercase; --label-ls: 0.16em; --label-w: 700;""",
  "warm": """
      --font-ui: 'Instrument Sans', system-ui, sans-serif;
      --font-mono: 'JetBrains Mono', ui-monospace, monospace;
      --r-lg: 14px; --r-md: 12px; --r-sm: 9px;
      --label-tt: uppercase; --label-ls: 0.13em; --label-w: 600;""",
  "brutalist": """
      --font-ui: 'Archivo', system-ui, sans-serif;
      --font-mono: 'IBM Plex Mono', ui-monospace, monospace;
      --r-lg: 0px; --r-md: 0px; --r-sm: 0px;
      --card-border: 2px solid var(--line);
      --chrome-border: 2px solid var(--line);
      --label-tt: uppercase; --label-ls: 0.2em; --label-w: 700;""",
}


def _terminal(theme):
    bg, fg, slots = TERM[theme]
    ansi = " ".join(f"--ansi-{i}: {c};" for i, c in enumerate(slots))
    return f"      --term-bg: {bg}; --term-fg: {fg};\n      {ansi}"


def _themes():
    out = []
    for theme in DARK:
        out.append(f'    [data-theme="{theme}"] {{\n      {DARK[theme]}{SHAPE[theme]}\n{_terminal(theme)}\n    }}\n')
    # An explicit ground wins; "system" defers to the OS. Both spellings resolve to one block.
    for theme in LIGHT:
        sel = f'[data-theme="{theme}"][data-ground="light"]'
        out.append(f'    {sel} {{\n      {LIGHT[theme]}\n    }}\n')
        out.append(
            '    @media (prefers-color-scheme: light) {\n'
            f'      [data-theme="{theme}"][data-ground="system"] {{\n        {LIGHT[theme]}\n      }}\n'
            '    }\n'
        )
    return "\n".join(out)


THEME_CSS = """
    body { margin: 0; }
    a { color: var(--accent); text-decoration: none; }
    a:hover { color: var(--accent-hi); }
    * { box-sizing: border-box; }

    [data-theme] {
      --font-ui: 'Manrope', system-ui, sans-serif;
      --font-mono: 'IBM Plex Mono', ui-monospace, monospace;
      --r-lg: 18px; --r-md: 13px; --r-sm: 10px;
      --card-border: 1px solid transparent;
      --chrome-border: 1px solid var(--line);
      --label-tt: none; --label-ls: 0em; --label-w: 700;
    }

""" + _themes() + """
    .lbl { text-transform: var(--label-tt); letter-spacing: var(--label-ls); font-weight: var(--label-w); }
    .card { background: var(--surface); border: var(--card-border); border-radius: var(--r-lg); }
    .key  { background: var(--raise); border: var(--card-border); border-radius: var(--r-sm);
            font-family: var(--font-mono); color: var(--text); text-align: center; padding: 12px 0; }
    .keyon { background: var(--accent); color: var(--on-accent); border: var(--card-border);
             border-radius: var(--r-sm); font-family: var(--font-mono); text-align: center;
             padding: 12px 0; font-weight: 600; }
    /* The terminal font is JetBrains Mono NL under every theme, and so is the dark ground. */
    .term { background: var(--term-bg); color: var(--term-fg);
            font-family: 'JetBrains Mono', ui-monospace, monospace; }
"""

SHELL = """<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <script src="./support.js"></script>
</head>
<body>
<x-dc>
<helmet>
  <link rel="stylesheet" href="{fonts}">
  <style>{css}</style>
</helmet>
<div data-theme="{{{{theme}}}}" data-ground="{{{{ground}}}}" style="width: {w}px; height: {h}px; background: var(--bg); color: var(--text); font-family: var(--font-ui); display: flex; flex-direction: column; overflow: hidden;">
{body}
</div>
</x-dc>
<script data-dc-script data-props='{{"theme": {{"editor": "enum", "options": ["soft", "phosphor", "warm", "brutalist"], "default": "soft", "section": "Theme"}}, "ground": {{"editor": "enum", "options": ["system", "dark", "light"], "default": "dark", "section": "Theme"}}, "$preview": {{"width": {w}, "height": {h}}}}}'>
class Component extends DCLogic {{
  renderVals() {{
    return {{ theme: this.props.theme ?? 'soft', ground: this.props.ground ?? 'dark' }};
  }}
}}
</script>
</body>
</html>
"""

SIZES = {
    "Main":                  (390, 844),
    "Pane-Portrait":         (390, 844),
    "Conversation-Portrait": (390, 844),
    "Pane-Typing":           (390, 470),
    "Zoom-Control":          (390, 844),
    "New-Sheet":             (390, 844),
    "Setup":                 (390, 844),
    "Pane-Landscape":        (844, 390),
    "Desktop":              (1440, 900),
    "Desktop-Mosaic":       (1440, 900),
    "Tokens":                (940, 790),
}

def main():
    names = sys.argv[1:] or sorted(SIZES)
    for name in names:
        w, h = SIZES[name]
        body = (PARTS / f"{name}.html").read_text().rstrip("\n")
        out = HERE / f"{name}.dc.html"
        out.write_text(SHELL.format(fonts=FONTS, css=THEME_CSS, w=w, h=h, body=body))
        print(f"built {out.name}  {w}x{h}")

if __name__ == "__main__":
    main()
