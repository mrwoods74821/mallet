"""Draws Mallet's icon: a white mallet on a black circle.

One set of shapes makes the SVG master and every bitmap. Bitmaps are drawn 16 times too
large and scaled down, so edges are smooth at every size. Run from anywhere:

    python branding/make_icons.py

Writes mallet.svg, mallet-glyph.svg, mallet-<size>.png and mallet.ico next to this file.
Needs Pillow.
"""

import math
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent

# The mallet standing upright on a 256 x 256 canvas, head at the top: (x0, y0, x1, y1, corner radius).
# The head is a body between two taller end caps, with a gap on each side, so it reads as a
# wooden mallet rather than a hammer. The handle widens slightly into a grip.
SHAPES = [
    (50, 32, 72, 100, 7),     # left end cap
    (184, 32, 206, 100, 7),   # right end cap
    (78, 39, 178, 93, 6),     # body of the head
    (116, 93, 140, 228, 12),  # handle
    (112, 176, 144, 232, 16), # grip
]
# At 24 pixels and below the gaps blur away and a thin handle turns grey, so small icons
# use the same outline without gaps and with a thicker handle.
SMALL_SHAPES = [
    (48, 30, 76, 102, 9),     # left end cap
    (180, 30, 208, 102, 9),   # right end cap
    (70, 38, 186, 94, 6),     # body of the head
    (110, 90, 146, 232, 18),  # handle and grip
]
SMALL_SIZE = 24
# The glyph is tilted so the head points to the top right.
TILT = 45
# How far the glyph may reach from the centre, as a share of the circle's radius.
REACH = 0.80
SUPERSAMPLE = 16
SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
PNG_SIZES = [32, 64, 128, 256, 512, 1024]


def glyph_frame(shapes=SHAPES):
    """The scale and centre that fit the tilted glyph inside the circle."""
    corners = [(x, y) for x0, y0, x1, y1, _ in shapes for x in (x0, x1) for y in (y0, y1)]
    centre_x = (min(x for x, _ in corners) + max(x for x, _ in corners)) / 2
    centre_y = (min(y for _, y in corners) + max(y for _, y in corners)) / 2
    reach = max(math.hypot(x - centre_x, y - centre_y) for x, y in corners)
    return 128 * REACH / reach, centre_x, centre_y


def render(size, circle=True):
    """The icon at `size` pixels, or with `circle` off just the white glyph."""
    big = size * SUPERSAMPLE
    shapes = SMALL_SHAPES if size <= SMALL_SIZE else SHAPES
    scale, centre_x, centre_y = glyph_frame(shapes)
    unit = big / 256
    glyph = Image.new("L", (big, big), 0)
    draw = ImageDraw.Draw(glyph)
    for x0, y0, x1, y1, radius in shapes:
        box = [
            (128 + (x0 - centre_x) * scale) * unit,
            (128 + (y0 - centre_y) * scale) * unit,
            (128 + (x1 - centre_x) * scale) * unit,
            (128 + (y1 - centre_y) * scale) * unit,
        ]
        draw.rounded_rectangle(box, radius=radius * scale * unit, fill=255)
    glyph = glyph.rotate(-TILT, resample=Image.BICUBIC, center=(big / 2, big / 2))

    image = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    if circle:
        disc = Image.new("L", (big, big), 0)
        ImageDraw.Draw(disc).ellipse([0, 0, big - 1, big - 1], fill=255)
        image.paste((0, 0, 0, 255), mask=disc)
    image.paste((255, 255, 255, 255), mask=glyph)
    return image.resize((size, size), Image.LANCZOS)


def svg(circle=True):
    """The same drawing as an SVG, for the README and anywhere that scales."""
    scale, centre_x, centre_y = glyph_frame()
    rects = "\n".join(
        f'    <rect x="{(x0 - centre_x) * scale + 128:.2f}" y="{(y0 - centre_y) * scale + 128:.2f}" '
        f'width="{(x1 - x0) * scale:.2f}" height="{(y1 - y0) * scale:.2f}" rx="{radius * scale:.2f}"/>'
        for x0, y0, x1, y1, radius in SHAPES
    )
    background = '  <circle cx="128" cy="128" r="128" fill="#000"/>\n' if circle else ""
    fill = "#fff" if circle else "currentColor"
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="256" height="256">\n'
        "  <title>Mallet</title>\n"
        f"{background}"
        f'  <g fill="{fill}" transform="rotate({TILT} 128 128)">\n{rects}\n  </g>\n'
        "</svg>\n"
    )


def main():
    (HERE / "mallet.svg").write_text(svg(), encoding="utf-8")
    (HERE / "mallet-glyph.svg").write_text(svg(circle=False), encoding="utf-8")
    for size in PNG_SIZES:
        render(size).save(HERE / f"mallet-{size}.png")
    icons = [render(size) for size in SIZES]
    icons[-1].save(HERE / "mallet.ico", sizes=[(size, size) for size in SIZES], append_images=icons[:-1])
    print("wrote", ", ".join(sorted(path.name for path in HERE.iterdir() if path.suffix in {".svg", ".png", ".ico"})))


if __name__ == "__main__":
    main()
