#!/usr/bin/env python3
"""重复生成 TokenMeter App / 菜单栏 / Windows 托盘图标。

母图语言：六边形信号孔径 + 两段非对称配额轨道。
- 青色轨道：实时额度信号
- 铜金轨道：账户价值/余额
- 图形在纯单色下仍成立，可作为 macOS template image
"""

from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter
import math

OUT = Path(__file__).resolve().parents[1] / "src-tauri" / "icons"
OUT.mkdir(parents=True, exist_ok=True)

INK = (4, 8, 13)
CYAN = (40, 218, 225)
CYAN_HI = (99, 238, 237)
COPPER = (207, 126, 65)
GOLD = (238, 184, 105)


def mix(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def rounded_mask(size, inset, radius):
    mask = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(mask)
    draw.rounded_rectangle(
        (inset, inset, size - inset, size - inset), radius=radius, fill=255
    )
    return mask


def polygon_points(cx, cy, radius, sides=6, rotation=-90):
    return [
        (
            cx + radius * math.cos(math.radians(rotation + i * 360 / sides)),
            cy + radius * math.sin(math.radians(rotation + i * 360 / sides)),
        )
        for i in range(sides)
    ]


def arc_point(cx, cy, radius, angle):
    radians = math.radians(angle)
    return cx + radius * math.cos(radians), cy + radius * math.sin(radians)


def draw_arc(draw, bbox, start, end, width, start_color, end_color, steps=150):
    span = end - start
    for i in range(steps):
        t = i / max(steps - 1, 1)
        color = mix(start_color, end_color, t)
        a0 = start + span * i / steps
        a1 = start + span * (i + 1.6) / steps
        draw.arc(bbox, a0, a1, fill=color + (255,), width=width)


def draw_mark(layer, size, colored=True, glow=False):
    draw = ImageDraw.Draw(layer)
    cx = cy = size / 2
    outer_radius = size * 0.292
    outer_width = max(2, round(size * 0.043))
    bbox = (
        cx - outer_radius,
        cy - outer_radius,
        cx + outer_radius,
        cy + outer_radius,
    )
    cyan_a = CYAN_HI if glow else CYAN
    copper_a = GOLD if glow else COPPER
    if colored:
        draw_arc(draw, bbox, 154, 275, outer_width, cyan_a, CYAN, 140)
        draw_arc(draw, bbox, -26, 92, outer_width, GOLD, copper_a, 140)
    else:
        draw.arc(bbox, 154, 275, fill=(0, 0, 0, 255), width=outer_width)
        draw.arc(bbox, -26, 92, fill=(0, 0, 0, 255), width=outer_width)

    node_radius = size * 0.026
    for angle, color in [(154, cyan_a), (92, copper_a)]:
        x, y = arc_point(cx, cy, outer_radius, angle)
        draw.ellipse(
            (x - node_radius, y - node_radius, x + node_radius, y + node_radius),
            fill=(color if colored else (0, 0, 0)) + (255,),
        )

    hex_outer = polygon_points(cx, cy, size * 0.193)
    hex_inner = polygon_points(cx, cy, size * 0.132)
    hex_width = max(2, round(size * 0.038))
    outline = GOLD if colored else (0, 0, 0)
    draw.line(hex_outer + [hex_outer[0]], fill=outline + (255,), width=hex_width, joint="curve")
    if colored:
        draw.line(
            hex_inner + [hex_inner[0]],
            fill=(28, 101, 112, 190),
            width=max(1, round(hex_width * 0.5)),
            joint="curve",
        )
    core = polygon_points(cx, cy, size * 0.078)
    draw.polygon(core, fill=(CYAN_HI if colored else (0, 0, 0)) + (255,))


def app_icon(size, supersample=4):
    s = size * supersample
    mask = rounded_mask(s, round(s * 0.035), round(s * 0.215))

    background = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    pixels = background.load()
    for y in range(s):
        t = y / max(s - 1, 1)
        row = mix((25, 31, 42), (5, 8, 14), t)
        for x in range(s):
            radial = max(0.0, 1.0 - math.hypot(x - s * 0.42, y - s * 0.36) / (s * 0.72))
            tint = mix(row, (21, 55, 61), radial * 0.12)
            pixels[x, y] = tint + (255,)
    background.putalpha(mask)

    glow = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    draw_mark(glow, s, colored=True, glow=True)
    glow = glow.filter(ImageFilter.GaussianBlur(max(2, round(s * 0.035))))
    glow.putalpha(Image.eval(glow.getchannel("A"), lambda value: round(value * 0.38)))
    background.alpha_composite(glow)

    mark = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    draw_mark(mark, s, colored=True)
    background.alpha_composite(mark)

    border = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    ImageDraw.Draw(border).rounded_rectangle(
        (s * 0.045, s * 0.045, s * 0.955, s * 0.955),
        radius=s * 0.205,
        outline=(126, 222, 228, 45),
        width=max(2, round(s * 0.007)),
    )
    background.alpha_composite(border)
    return background.resize((size, size), Image.Resampling.LANCZOS)


def mark_icon(size, colored, supersample=4):
    s = size * supersample
    image = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    draw_mark(image, s, colored=colored)
    return image.resize((size, size), Image.Resampling.LANCZOS)


for icon_size in [32, 128, 256, 512, 1024]:
    app_icon(icon_size).save(OUT / f"{icon_size}x{icon_size}.png")
master = app_icon(1024)
app_icon(512).save(OUT / "icon.png")
master.save(
    OUT / "icon.ico",
    format="ICO",
    sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
)
master.save(OUT / "icon.icns", format="ICNS")

menubar = mark_icon(512, colored=False)
menubar.save(OUT / "menubar.png")
(OUT / "menubar.rgba").write_bytes(mark_icon(64, colored=False).tobytes())
(OUT / "tray_color.rgba").write_bytes(mark_icon(64, colored=True).tobytes())

print("TokenMeter app / menubar / Windows tray icons generated")
