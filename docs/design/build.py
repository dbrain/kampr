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

    [data-theme="soft"] {
      --bg: #0e0f13; --bar: #12141a; --surface: #171a21; --surface-2: #090a0d;
      --raise: #22262f; --line: #1e2129;
      --text: #e8eaf0; --dim: #8d93a3; --mute: #545a68;
      --accent: #6ea8fe; --accent-hi: #9cc4ff; --on-accent: #0e0f13;
      --accent-soft: #16203050;
      --blocked: #ff6b6b; --blocked-bg: #2c1b20; --working: #ffc857;
      --idle: #5b6172; --done: #58d68d;
    }

    [data-theme="phosphor"] {
      --bg: #0a0b0a; --bar: #0c0e0c; --surface: #101210; --surface-2: #070807;
      --raise: #171a17; --line: #1d211d;
      --text: #cdd6cd; --dim: #6b756b; --mute: #4d554d;
      --accent: #f5c542; --accent-hi: #ffd968; --on-accent: #0a0b0a;
      --accent-soft: #16130a;
      --blocked: #ff5f56; --blocked-bg: #150d0c; --working: #f5c542;
      --idle: #4a534a; --done: #57c46b;
      --font-ui: 'JetBrains Mono', ui-monospace, monospace;
      --font-mono: 'JetBrains Mono', ui-monospace, monospace;
      --r-lg: 2px; --r-md: 2px; --r-sm: 2px;
      --card-border: 1px solid var(--line);
      --label-tt: uppercase; --label-ls: 0.16em; --label-w: 700;
    }

    [data-theme="warm"] {
      --bg: #12100e; --bar: #171412; --surface: #1a1714; --surface-2: #0d0b0a;
      --raise: #241f1b; --line: #221e1a;
      --text: #efe7dc; --dim: #8d8175; --mute: #5f574e;
      --accent: #e0a458; --accent-hi: #f0bd7c; --on-accent: #12100e;
      --accent-soft: #211a12;
      --blocked: #e0685a; --blocked-bg: #2a1613; --working: #e0a458;
      --idle: #4a423a; --done: #7fb069;
      --font-ui: 'Instrument Sans', system-ui, sans-serif;
      --font-mono: 'JetBrains Mono', ui-monospace, monospace;
      --r-lg: 14px; --r-md: 12px; --r-sm: 9px;
      --label-tt: uppercase; --label-ls: 0.13em; --label-w: 600;
    }

    [data-theme="brutalist"] {
      --bg: #000000; --bar: #000000; --surface: #000000; --surface-2: #000000;
      --raise: #000000; --line: #ffffff;
      --text: #ffffff; --dim: #8a8a8a; --mute: #5c5c5c;
      --accent: #ff2b1f; --accent-hi: #ff6a60; --on-accent: #000000;
      --accent-soft: #000000;
      --blocked: #ff2b1f; --blocked-bg: #000000; --working: #ffffff;
      --idle: #333333; --done: #ffffff;
      --font-ui: 'Archivo', system-ui, sans-serif;
      --font-mono: 'IBM Plex Mono', ui-monospace, monospace;
      --r-lg: 0px; --r-md: 0px; --r-sm: 0px;
      --card-border: 2px solid var(--line);
      --chrome-border: 2px solid var(--line);
      --label-tt: uppercase; --label-ls: 0.2em; --label-w: 700;
    }

    .lbl { text-transform: var(--label-tt); letter-spacing: var(--label-ls); font-weight: var(--label-w); }
    .card { background: var(--surface); border: var(--card-border); border-radius: var(--r-lg); }
    .key  { background: var(--raise); border: var(--card-border); border-radius: var(--r-sm);
            font-family: var(--font-mono); color: var(--text); text-align: center; padding: 12px 0; }
    .keyon { background: var(--accent); color: var(--on-accent); border: var(--card-border);
             border-radius: var(--r-sm); font-family: var(--font-mono); text-align: center;
             padding: 12px 0; font-weight: 600; }
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
<div data-theme="{{{{theme}}}}" style="width: {w}px; height: {h}px; background: var(--bg); color: var(--text); font-family: var(--font-ui); display: flex; flex-direction: column; overflow: hidden;">
{body}
</div>
</x-dc>
<script data-dc-script data-props='{{"theme": {{"editor": "enum", "options": ["soft", "phosphor", "warm", "brutalist"], "default": "soft", "section": "Theme"}}, "$preview": {{"width": {w}, "height": {h}}}}}'>
class Component extends DCLogic {{
  renderVals() {{
    return {{ theme: this.props.theme ?? 'soft' }};
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
    "Tokens":                (940, 620),
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
