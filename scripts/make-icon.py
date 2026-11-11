"""Generate the app icons (rapidcap.icns, rapidcap.ico) from the product mark.

The mark: two viewfinder corners, top-left and bottom-right, leaning forward,
around a red record dot - select, record, fast. Electric blue on a graphite
tile, drawn on the 1024 macOS icon grid (an 824 tile, 100 in from each edge).
The menu bar draws the same corners and dot as a template image
(macos/Sources/RapidCap/StatusMark.swift); keep the two in step.

Run after changing the mark:  python scripts/make-icon.py   (needs Pillow)
"""

from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

GRID = 1024
SUPER = 4  # supersampling for the antialiased shapes

TILE_TOP = (40, 44, 54)
TILE_BOTTOM = (10, 11, 15)
BLUE = ((90, 200, 250), (59, 91, 255))
RED = ((255, 99, 99), (255, 59, 48))

# The corners: stroke width, arm length, and the box they mark.
STROKE = 74
ARM = 250
BOX = (262, 282, 762, 742)
LEAN = 0.18  # horizontal shift per unit of height: the forward lean
DOT = 70


def gradient(top, bottom, angle=90):
    """`top` to `bottom` along `angle` degrees (90 is straight down)."""
    ramp = Image.linear_gradient("L").rotate(90 - angle, resample=Image.BICUBIC)
    ramp = ramp.resize((GRID, GRID), Image.BICUBIC)
    return Image.composite(Image.new("RGBA", (GRID, GRID), bottom + (255,)),
                           Image.new("RGBA", (GRID, GRID), top + (255,)), ramp)


def shape(draw):
    big = Image.new("L", (GRID * SUPER, GRID * SUPER), 0)
    draw(ImageDraw.Draw(big), SUPER)
    return big.resize((GRID, GRID), Image.LANCZOS)


def tile_mask(inset, radius):
    return shape(lambda d, k: d.rounded_rectangle(
        [inset * k, inset * k, (GRID - inset) * k, (GRID - inset) * k], radius * k, fill=255))


def lean(mask):
    """Shear about the centre so the top moves right: the mark leans forward."""
    centre = GRID / 2
    return mask.transform((GRID, GRID), Image.AFFINE, (1, LEAN, -LEAN * centre, 0, 1, 0),
                          resample=Image.BICUBIC)


def corners_mask():
    x0, y0, x1, y1 = BOX
    r = STROKE / 2
    tall = ARM * 0.8

    def draw(d, k):
        d.rounded_rectangle([x0 * k, y0 * k, (x0 + ARM) * k, (y0 + STROKE) * k], r * k, fill=255)
        d.rounded_rectangle([x0 * k, y0 * k, (x0 + STROKE) * k, (y0 + tall) * k], r * k, fill=255)
        d.rounded_rectangle([(x1 - ARM) * k, (y1 - STROKE) * k, x1 * k, y1 * k], r * k, fill=255)
        d.rounded_rectangle([(x1 - STROKE) * k, (y1 - tall) * k, x1 * k, y1 * k], r * k, fill=255)

    return lean(shape(draw))


def dot_mask():
    c = GRID / 2
    return shape(lambda d, k: d.ellipse([(c - DOT) * k, (c - DOT) * k, (c + DOT) * k, (c + DOT) * k], fill=255))


def paint(icon, mask, colours):
    # A soft glow of the deeper colour under the shape, then the shape.
    glow = Image.new("RGBA", (GRID, GRID), colours[1] + (0,))
    glow.putalpha(mask.point(lambda v: int(v * 0.55)))
    icon.alpha_composite(glow.filter(ImageFilter.GaussianBlur(28)))
    face = gradient(colours[0], colours[1], 60)
    face.putalpha(mask)
    icon.alpha_composite(face)


def mark():
    icon = Image.new("RGBA", (GRID, GRID), (0, 0, 0, 0))
    tile = tile_mask(100, 186)

    shadow = Image.new("RGBA", (GRID, GRID), (0, 0, 0, 0))
    shadow.putalpha(tile.point(lambda v: int(v * 0.35)))
    icon.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(20)), (0, 14))

    face = gradient(TILE_TOP, TILE_BOTTOM)
    face.putalpha(tile)
    icon.alpha_composite(face)

    # A hairline of light round the edge and a faint sheen at the top, the
    # way macOS icons catch the light.
    rim = Image.new("RGBA", (GRID, GRID), (255, 255, 255, 0))
    rim.putalpha(ImageChops.subtract(tile, tile_mask(103, 183)).point(lambda v: int(v * 0.35)))
    icon.alpha_composite(rim)
    sheen = Image.new("RGBA", (GRID, GRID), (255, 255, 255, 0))
    fade = Image.linear_gradient("L").resize((GRID, GRID)).point(lambda v: int(max(0, 120 - v) * 0.18))
    sheen.putalpha(ImageChops.multiply(fade, tile))
    icon.alpha_composite(sheen)

    paint(icon, corners_mask(), BLUE)
    paint(icon, dot_mask(), RED)
    return icon


def main():
    assets = Path(__file__).resolve().parent.parent / "crates" / "desktop" / "assets"
    icon = mark()
    icns = assets / "rapidcap.icns"
    icon.save(icns)
    ico = assets / "rapidcap.ico"
    icon.save(ico, sizes=[(16, 16), (20, 20), (24, 24), (32, 32), (40, 40), (48, 48), (64, 64), (128, 128),
                          (256, 256)])
    for path in (icns, ico):
        print(f"{path}  {path.stat().st_size} bytes")


if __name__ == "__main__":
    main()
