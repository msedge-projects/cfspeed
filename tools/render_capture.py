"""Replay ANSI/CUP output from a TUI capture and print the final screen grid."""
import re
import sys

ROWS, COLS = 45, 130

data = open(sys.argv[1], "rb").read().decode("utf-8", "replace")

grid = [[" "] * COLS for _ in range(ROWS)]
row = col = 0

# Split into tokens we care about; everything else is consumed as text.
token = re.compile(r"\x1b\[([0-9;?]*)([A-Za-z])|\x1b\][^\x07]*\x07")

pos = 0
while pos < len(data):
    m = token.search(data, pos)
    if not m:
        chunk = data[pos:]
        for ch in chunk:
            if ch == "\n":
                row = min(row + 1, ROWS - 1)
            elif ch == "\r":
                col = 0
            elif ch == "\x08":
                col = max(0, col - 1)
            elif ch >= " ":
                if row < ROWS and col < COLS:
                    grid[row][col] = ch
                col += 1
                if col >= COLS:
                    col = COLS - 1
        break

    chunk = data[pos:m.start()]
    for ch in chunk:
        if ch == "\n":
            row = min(row + 1, ROWS - 1)
        elif ch == "\r":
            col = 0
        elif ch == "\x08":
            col = max(0, col - 1)
        elif ch >= " ":
            if row < ROWS and col < COLS:
                grid[row][col] = ch
            col += 1
            if col >= COLS:
                col = COLS - 1

    params, final = m.group(1), m.group(2)
    if final == "H":
        nums = [int(x) for x in params.split(";") if x != ""]
        r = nums[0] if len(nums) > 0 else 1
        c = nums[1] if len(nums) > 1 else 1
        row, col = r - 1, c - 1
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
        grid = [[" "] * COLS for _ in range(ROWS)]
    # SGR (m), mode sets (h/l), charset, save/restore: ignore.
    pos = m.end()

# Re-read with a decoder that never substitutes, so stray bytes from colour
# handling surface as explicit markers instead of silently eating content.
def _audit(path):
    raw = open(path, "rb").read()
    text = raw.decode("utf-8", "replace")
    if "�" in text:
        print(f"[warn] {text.count(chr(0xFFFD))} undecodable byte(s) in capture",
              file=sys.stderr)

for line in grid:
    print("".join(line).rstrip())