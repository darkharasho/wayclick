#!/usr/bin/env python3
"""Generate the wayclick app icon, then run `cargo tauri icon assets/icon.png`.

The icon is the app's cadence signature: steel pulse bars framing a tall
sodium-amber playhead (the "live beat") on a gradient squircle. Requires Pillow.
"""

from PIL import Image, ImageDraw, ImageFilter

SS = 4
S = 1024 * SS
STEEL = (132, 150, 176, 255)
AMBER = (255, 154, 60, 255)


def main(out="assets/icon.png"):
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))

    # gradient squircle tile
    grad = Image.new("RGB", (1, S))
    for y in range(S):
        t = y / S
        grad.putpixel(
            (0, y),
            (
                int(0x16 * (1 - t) + 0x0E * t),
                int(0x18 * (1 - t) + 0x0F * t),
                int(0x20 * (1 - t) + 0x15 * t),
            ),
        )
    grad = grad.resize((S, S))
    mask = Image.new("L", (S, S), 0)
    m, rad = int(S * 0.055), int(S * 0.235)
    ImageDraw.Draw(mask).rounded_rectangle([m, m, S - m, S - m], radius=rad, fill=255)
    tile = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    tile.paste(grad, (0, 0), mask)
    ImageDraw.Draw(tile).rounded_rectangle(
        [m, m, S - m, S - m], radius=rad, outline=(44, 49, 60, 255), width=int(S * 0.007)
    )
    im = Image.alpha_composite(im, tile)

    cx = cy = S // 2

    # soft amber glow behind the playhead
    glow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(glow).ellipse(
        [cx - int(S * 0.17), cy - int(S * 0.30), cx + int(S * 0.17), cy + int(S * 0.30)],
        fill=(255, 154, 60, 150),
    )
    im = Image.alpha_composite(im, glow.filter(ImageFilter.GaussianBlur(S * 0.055)))

    # cadence bars: 4 steel + 1 tall amber playhead (centered peak)
    d = ImageDraw.Draw(im)
    bars = [(-2, 0.32, STEEL), (-1, 0.46, STEEL), (0, 0.80, AMBER), (1, 0.46, STEEL), (2, 0.32, STEEL)]
    bw, gap = int(S * 0.072), int(S * 0.052)
    for idx, h, col in bars:
        bh = int(S * h)
        x = cx + idx * (bw + gap) - bw // 2
        d.rounded_rectangle([x, cy - bh // 2, x + bw, cy + bh // 2], radius=bw // 2, fill=col)

    im.resize((1024, 1024), Image.LANCZOS).save(out)
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
