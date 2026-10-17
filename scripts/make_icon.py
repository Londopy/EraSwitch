"""Draws assets/icon.ico (and a preview PNG) for EraSwitch.

The mark: a rounded square split on the diagonal, Windows 11 blue on top and
XP start-button green below, with a white E on it. Each size is drawn at 8x
and scaled down, so the small ones stay clean.

    python scripts/make_icon.py

Needs Pillow (pip install pillow). Only used to regenerate the icon; the app
and its build don't depend on it.
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parent.parent
OUT_ICO = ROOT / "assets" / "icon.ico"
OUT_PNG = ROOT / "assets" / "icon-256.png"

SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]
SS = 8  # supersampling factor

# Windows 11 accent blue, lighter at the top-left corner
BLUE_LIGHT = (38, 140, 230)
BLUE_DARK = (0, 92, 184)
# the XP start button's greens
GREEN_LIGHT = (95, 175, 70)
GREEN_DARK = (46, 128, 38)
WHITE = (255, 255, 255, 255)


def lerp(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def diagonal_gradient(n, top, bottom):
    """n x n image fading from `top` (top-left) to `bottom` (bottom-right)."""
    g = Image.new("RGB", (n, n))
    px = g.load()
    for y in range(n):
        for x in range(n):
            px[x, y] = lerp(top, bottom, (x + y) / (2 * (n - 1)))
    return g


def draw(size):
    n = size * SS
    # small sizes get a bit less margin and corner so they read at a glance
    margin = round(n * (0.03 if size <= 24 else 0.06))
    radius = round(n * (0.20 if size <= 24 else 0.22))
    box = (margin, margin, n - margin - 1, n - margin - 1)

    # the rounded square's shape
    mask = Image.new("L", (n, n), 0)
    ImageDraw.Draw(mask).rounded_rectangle(box, radius=radius, fill=255)

    # the two eras, split on the diagonal (bottom-left to top-right)
    blue = diagonal_gradient(n, BLUE_LIGHT, BLUE_DARK)
    green = diagonal_gradient(n, GREEN_LIGHT, GREEN_DARK)
    split = Image.new("L", (n, n), 0)
    ImageDraw.Draw(split).polygon([(n, 0), (n, n), (0, n)], fill=255)
    body = Image.composite(green, blue, split).convert("RGBA")

    # a thin light seam along the diagonal, only where there's room for it
    if size >= 32:
        seam = Image.new("L", (n, n), 0)
        w = max(SS, round(n * 0.012))
        ImageDraw.Draw(seam).line([(0, n), (n, 0)], fill=110, width=w)
        body = Image.composite(Image.new("RGBA", (n, n), WHITE), body, seam)

    icon = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    icon.paste(body, (0, 0), mask)

    # the E: a spine and three arms, sized to the square
    inner = box[2] - box[0]
    left = box[0] + round(inner * 0.27)
    top = box[1] + round(inner * 0.20)
    height = round(inner * 0.60)
    width = round(inner * 0.46)
    stroke = round(inner * (0.17 if size <= 24 else 0.14))
    middle = round(width * 0.82)

    e = Image.new("L", (n, n), 0)
    d = ImageDraw.Draw(e)
    d.rectangle([left, top, left + stroke, top + height], fill=255)
    d.rectangle([left, top, left + width, top + stroke], fill=255)
    mid_y = top + (height - stroke) // 2
    d.rectangle([left, mid_y, left + middle, mid_y + stroke], fill=255)
    d.rectangle([left, top + height - stroke, left + width, top + height], fill=255)

    # a soft shadow under the E where it has room to show
    if size >= 48:
        shadow = e.filter(ImageFilter.GaussianBlur(n * 0.012))
        shadow = Image.eval(shadow, lambda v: v * 0.35)
        dark = Image.new("RGBA", (n, n), (0, 30, 60, 255))
        offset = round(n * 0.012)
        shadow_layer = Image.new("RGBA", (n, n), (0, 0, 0, 0))
        shadow_layer.paste(dark, (offset, offset), shadow)
        icon = Image.alpha_composite(icon, shadow_layer)

    icon.paste(Image.new("RGBA", (n, n), WHITE), (0, 0), e)
    return icon.resize((size, size), Image.LANCZOS)


def main():
    OUT_ICO.parent.mkdir(parents=True, exist_ok=True)
    images = [draw(s) for s in SIZES]
    biggest = images[-1]
    # Pillow writes one entry per size listed, each from its own drawing
    biggest.save(OUT_ICO, format="ICO", sizes=[(s, s) for s in SIZES], append_images=images[:-1])
    biggest.save(OUT_PNG)
    print(f"wrote {OUT_ICO.relative_to(ROOT)} ({', '.join(map(str, SIZES))} px) and {OUT_PNG.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
