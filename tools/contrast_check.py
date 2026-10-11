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


SURFACE = "#17181C"
SURFACE_ALT = "#1F2126"

PALETTE = {
    "TEXT": "#C4C7CE",
    "TEXT_BRIGHT": "#EDEFF2",
    "TEXT_DIM": "#8E939C",
    "BORDER": "#343842",
    "ACCENT_DOWN": "#5CC0A8",
    "ACCENT_UP": "#E0A458",
    "ACCENT_LAT": "#9B8FD4",
    "GRADE_A": "#6FBF73",
    "GRADE_C": "#E0C060",
    "GRADE_F": "#DC6B62",
}

print(f"surface {SURFACE} / alt {SURFACE_ALT}\n")
print(f"{'token':<16}{'hex':<10}{'vs surface':>12}{'vs alt':>10}  verdict")
print("-" * 62)
for name, hexv in PALETTE.items():
    c1 = contrast(hexv, SURFACE)
    c2 = contrast(hexv, SURFACE_ALT)
    # Body text needs 4.5:1; large text and borders only 3:1.
    floor = 3.0 if "BORDER" in name else 4.5
    ok = "PASS" if min(c1, c2) >= floor else "FAIL"
    print(f"{name:<16}{hexv:<10}{c1:>11.2f}:1{c2:>9.2f}:1  {ok}")

print()
adj = contrast(PALETTE["TEXT_BRIGHT"], PALETTE["ACCENT_DOWN"])
print(f"bright-on-down-accent {adj:.2f}:1")
print(f"border visibility vs surface {contrast(PALETTE['BORDER'], SURFACE):.2f}:1 (decorative)")