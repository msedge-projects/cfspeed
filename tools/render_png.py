"""Replay a TUI capture and render it to a PNG with real font and colour.

A text grid cannot show what a chart actually looks like, and the user's
terminal is the only thing that matters. This replays the capture while
tracking SGR foreground/background state, then draws each cell with the real
Consolas face so braille strokes render the way the terminal draws them.

Usage: render_png.py <capture> <out.png>
"""
import re
import sys

from PIL import Image, ImageDraw, ImageFont

ROWS, COLS = 45, 130
CELL_W, CELL_H = 9, 19
FONT_SIZE = 16
PAD = 8

capture, out_path = sys.argv[1], sys.argv[2]
# Optional third argument: composite over this colour instead of leaving the
# background transparent, so the chart can be checked against a light terminal
# theme as well as a dark one.
over = sys.argv[3] if len(sys.argv) > 3 else None

# xterm 16-colour palette, tuned to match a typical dark terminal.
BASE = {
    0: (0x1A, 0x1B, 0x26), 1: (0xF7, 0x76, 0x8E), 2: (0x9E, 0xCE, 0x6A),
    3: (0xE0, 0xAF, 0x68), 4: (0x7A, 0xAA, 0xFE), 5: (0xBB, 0x9A, 0xF7),
    6: (0x7D, 0xCF, 0xFF), 7: (0xC0, 0xCA, 0xF5),
}
BRIGHT = {
    0: (0x41, 0x41, 0x52), 1: (0xFF, 0x9E, 0xB0), 2: (0xB9, 0xF2, 0x7C),
    3: (0xFF, 0xC7, 0x77), 4: (0x9A, 0xC7, 0xFF), 5: (0xD2, 0xB4, 0xFF),
    6: (0xA4, 0xE0, 0xFF), 7: (0xE6, 0xEA, 0xFF),
}
DEFAULT_FG = (0xC0, 0xCA, 0xF5)
DEFAULT_BG = (0x1A, 0x1B, 0x26)

data = open(capture, "rb").read().decode("utf-8", "replace")

sym = [[" "] * COLS for _ in range(ROWS)]
fg = [[None] * COLS for _ in range(ROWS)]
bg = [[None] * COLS for _ in range(ROWS)]
row = col = 0
cur_fg = cur_bg = None
bold = False

tok = re.compile(r"\x1b\[([0-9;?]*)([A-Za-z])|\x1b\][^\x07]*\x07")


def put(ch):
    global col
    if 0 <= row < ROWS and 0 <= col < COLS:
        sym[row][col] = ch
        fg[row][col] = cur_fg
        bg[row][col] = cur_bg
    col += 1
    if col >= COLS:
        col = COLS - 1


pos = 0
while pos < len(data):
    m = tok.search(data, pos)
    chunk = data[pos : m.start()] if m else data[pos:]
    for ch in chunk:
        if ch == "\n":
            row = min(row + 1, ROWS - 1)
        elif ch == "\r":
            col = 0
        elif ch == "\x08":
            col = max(0, col - 1)
        elif ch >= " ":
            put(ch)
    if not m:
        break

    params, final = m.group(1), m.group(2)
    if final == "H":
        nums = [int(x) for x in params.split(";") if x != ""]
        row = (nums[0] if nums else 1) - 1
        col = (nums[1] if len(nums) > 1 else 1) - 1
    elif final == "A":
        row = max(0, row - int(params or 1))
    elif final == "B":
        row = min(ROWS - 1, row + int(params or 1))
    elif final == "C":
        col = min(COLS - 1, col + int(params or 1))
    elif final == "D":
        col = max(0, col - int(params or 1))
    elif final == "G":
        col = max(0, int(params or 1) - 1)
    elif final == "J":
        sym = [[" "] * COLS for _ in range(ROWS)]
        fg = [[None] * COLS for _ in range(ROWS)]
        bg = [[None] * COLS for _ in range(ROWS)]
    elif final == "m":
        ps = [int(x) for x in params.split(";") if x != ""] or [0]
        i = 0
        while i < len(ps):
            p = ps[i]
            if p == 0:
                cur_fg = cur_bg = None
                bold = False
            elif p == 1:
                bold = True
            elif p == 22:
                bold = False
            elif 30 <= p <= 37:
                cur_fg = BASE[p - 30]
            elif 90 <= p <= 97:
                cur_fg = BRIGHT[p - 90]
            elif 40 <= p <= 47:
                cur_bg = BASE[p - 40]
            elif 100 <= p <= 107:
                cur_bg = BRIGHT[p - 100]
            elif p == 39:
                cur_fg = None
            elif p == 49:
                cur_bg = None
            elif p in (38, 48) and i + 4 < len(ps) and ps[i + 1] == 2:
                col_rgb = tuple(ps[i + 2 : i + 5])
                if p == 38:
                    cur_fg = col_rgb
                else:
                    cur_bg = col_rgb
                i += 4
            elif p in (38, 48) and i + 2 < len(ps) and ps[i + 1] == 5:
                idx = ps[i + 2]
                tbl = BRIGHT if idx > 7 else BASE
                rgb = tbl.get(idx % 8, DEFAULT_FG)
                if p == 38:
                    cur_fg = rgb
                else:
                    cur_bg = rgb
                i += 2
            i += 1
    pos = m.end()

W = COLS * CELL_W + PAD * 2
H = ROWS * CELL_H + PAD * 2
# Transparent unless a colour was given. The app paints no background of its
# own, so anything opaque here would be the renderer lying about it.
base = tuple(int(over[i : i + 2], 16) for i in (1, 3, 5)) if over else None
img = Image.new("RGBA", (W, H), (base + (255,)) if base else (0, 0, 0, 0))
d = ImageDraw.Draw(img)
font = ImageFont.truetype(r"C:\Windows\Fonts\consola.ttf", FONT_SIZE)

for y in range(ROWS):
    for x in range(COLS):
        px, py = PAD + x * CELL_W, PAD + y * CELL_H
        b = bg[y][x]
        if b is not None:
            d.rectangle([px, py, px + CELL_W - 1, py + CELL_H - 1], fill=b + (255,))
        ch = sym[y][x]
        if ch != " ":
            d.text((px, py), ch, font=font, fill=cur_fg if False else (fg[y][x] or DEFAULT_FG))

img.save(out_path)
painted = sum(1 for y in range(ROWS) for x in range(COLS) if bg[y][x])
print(f"{out_path}: {W}x{H}, {painted} background-painted cells")