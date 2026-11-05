"""Draws Folio's icons for the mobile fork (an open book in the Encore colours) and writes them where the fork
picks them up: the launcher and notification icons into the Android resource overlay (android/app/folio-res, used
in place of the upstream ones by folio.gradle) and the in-app logo into static/Logo.png.

Run from abs-app/: python3 folio/make_icons.py (needs Pillow). The output is checked in; run it again only when
the drawing changes."""

import os

from PIL import Image, ImageDraw

BG = (0x16, 0x16, 0x17, 255)  # Encore --bg
FG = (0xF0, 0xEA, 0xD6, 255)  # Encore --text / --accent

# The book on a 108-unit square (the adaptive icon's canvas; the safe zone is the middle 66 units).
LEFT = [(30, 38), (52, 43), (52, 75), (30, 70)]
RIGHT = [(56, 43), (78, 38), (78, 70), (56, 75)]


def book(size, background, inset=0.0):
    """The book drawn on a [size] square, on [background] (None for clear), scaled into the middle when [inset] > 0."""
    s = size * 4  # draw large, then shrink, for smooth edges
    img = Image.new("RGBA", (s, s), background or (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    scale = s / 108 * (1 - inset)
    off = s * inset / 2
    for page in (LEFT, RIGHT):
        d.polygon([(off + x * scale, off + y * scale) for x, y in page], fill=FG)
    return img.resize((size, size), Image.LANCZOS)


def save(img, path):
    """Writes [img] to [path], making its folder."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    img.save(path, optimize=True)


def main():
    """Writes every icon."""
    res = "android/app/folio-res"
    for name, launcher in (("mdpi", 48), ("hdpi", 72), ("xhdpi", 96), ("xxhdpi", 144), ("xxxhdpi", 192)):
        adaptive = launcher * 9 // 4  # 108 dp against the 48 dp launcher icon
        save(book(adaptive, None), f"{res}/mipmap-{name}/ic_launcher_foreground.png")
        save(Image.new("RGBA", (adaptive, adaptive), BG), f"{res}/mipmap-{name}/ic_launcher_background.png")
        square = book(launcher, BG, inset=-0.3)
        save(square, f"{res}/mipmap-{name}/ic_launcher.png")
        mask = Image.new("L", (launcher * 4, launcher * 4), 0)
        ImageDraw.Draw(mask).ellipse((0, 0, launcher * 4 - 1, launcher * 4 - 1), fill=255)
        round_icon = Image.new("RGBA", (launcher, launcher), (0, 0, 0, 0))
        round_icon.paste(square, (0, 0), mask.resize((launcher, launcher), Image.LANCZOS))
        save(round_icon, f"{res}/mipmap-{name}/ic_launcher_round.png")
    save(book(162, BG, inset=-0.3), f"{res}/drawable/icon.png")
    save(book(256, None, inset=-0.3), "static/Logo.png")
    pages = " ".join("M" + " L".join(f"{x},{y}" for x, y in p) + " Z" for p in (LEFT, RIGHT))
    with open(f"{res}/drawable/icon_monochrome.xml", "w") as f:
        f.write(
            '<?xml version="1.0" encoding="utf-8"?>\n'
            "<!-- Folio: the notification icon (an open book); drawn by abs-app/folio/make_icons.py -->\n"
            '<vector xmlns:android="http://schemas.android.com/apk/res/android"\n'
            '    android:width="24dp"\n    android:height="24dp"\n'
            '    android:viewportWidth="108"\n    android:viewportHeight="108">\n'
            f'  <path android:fillColor="#FFFFFF" android:pathData="{pages}" />\n'
            "</vector>\n"
        )


if __name__ == "__main__":
    main()
