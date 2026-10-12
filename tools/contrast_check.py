"""Validate a dark-terminal palette for WCAG contrast.

Usage: python contrast_check.py
"""

def srgb_to_linear(c):
    c = c / 255.0
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def luminance(hex_str):
    h = hex_str.lstrip("#")
    r, g, b = (int(h[i:i + 2], 16) for i in (0, 2, 4))
    return 0.2126 * srgb_to_linear(r) + 0.7152 * srgb_to_linear(g) + 0.0722 * srgb_to_linear(b)


def contrast(a, b):
    la, lb = luminance(a), luminance(b)
    hi, lo = max(la, lb), min(la, lb)
    return (hi + 0.05) / (lo + 0.05)


# Tokyo Night, as shipped in btop's themes/tokyo-night.theme
SURFACE = "#1a1b26"

PALETTE = {
    "TEXT": "#a9b1d6",
    "TEXT_BRIGHT": "#c0caf5",
    "TEXT_DIM": "#9399b5",
    "BORDER": "#404040",
    "ACCENT_DOWN": "#9ece6a",
    "ACCENT_UP": "#e0af68",
    "ACCENT_LAT": "#7dcfff",
    "GRADE_A": "#9ece6a",
    "GRADE_C": "#e0af68",
    "GRADE_F": "#f7768e",
}

print(f"surface {SURFACE}\n")
print(f"{'token':<16}{'hex':<10}{'vs surface':>12}{'vs alt':>10}  verdict")
print("-" * 62)
for name, hexv in PALETTE.items():
    c1 = contrast(hexv, SURFACE)
    c2 = c1
    # Body text needs 4.5:1; large text and borders only 3:1.
    floor = 3.0 if "BORDER" in name else 4.5
    ok = "PASS" if min(c1, c2) >= floor else "FAIL"
    print(f"{name:<16}{hexv:<10}{c1:>11.2f}:1{c2:>9.2f}:1  {ok}")

print()
adj = contrast(PALETTE["TEXT_BRIGHT"], PALETTE["ACCENT_DOWN"])
print(f"bright-on-down-accent {adj:.2f}:1")
print(f"border visibility vs surface {contrast(PALETTE['BORDER'], SURFACE):.2f}:1 (decorative)")