"""Replay an ANSI capture and show which cells carry a background colour.

The chart body is painted with background colour and a blank glyph, so a plain
text replay renders it as empty. This tracks SGR state and prints `#` for any
cell with a non-default background, which is what the solid body actually is.
"""
import re
import sys

ROWS, COLS = 45, 130

data = open(sys.argv[1], "rb").read().decode("utf-8", "replace")

sym = [[" "] * COLS for _ in range(ROWS)]
bg = [[None] * COLS for _ in range(ROWS)]
row = col = 0
cur_bg = None

tok = re.compile(r"\x1b\[([0-9;?]*)([A-Za-z])|\x1b\][^\x07]*\x07")


def put(ch):
    global col
    if 0 <= row < ROWS and 0 <= col < COLS:
        sym[row][col] = ch
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
        bg = [[None] * COLS for _ in range(ROWS)]
    elif final == "m":
        ps = [int(x) for x in params.split(";") if x != ""] or [0]
        i = 0
        while i < len(ps):
            p = ps[i]
            if p in (0, 49):
                cur_bg = None
            elif p == 48 and i + 4 < len(ps) and ps[i + 1] == 2:
                cur_bg = tuple(ps[i + 2 : i + 5])
                i += 4
            elif p == 48 and i + 2 < len(ps) and ps[i + 1] == 5:
                cur_bg = ("idx", ps[i + 2])
                i += 2
            i += 1
    pos = m.end()

for y in range(ROWS):
    line = "".join("#" if bg[y][x] else sym[y][x] for x in range(COLS))
    print(line.rstrip())