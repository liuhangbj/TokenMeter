#!/usr/bin/env python3
"""用 PIL 直接绘制 TokenMeter 图标（零外部 C 库依赖，可重复生成）。
设计：仪表盘意象 —— 3/4 圆环（5段警示色渐变）+ 指针 + 轴心。
- App 图标：圆角深底 + 彩色表盘
- 菜单栏图标：纯黑剪影模板图（透明底，系统适配明暗）
"""
from PIL import Image, ImageDraw
import math, os

OUT = "src-tauri/icons"
os.makedirs(OUT, exist_ok=True)

# 5 段警示色
LV = ["#7ed79b", "#2ba471", "#e37318", "#d54941", "#9e2b25"]

def hex2rgb(h):
    h = h.lstrip("#")
    return tuple(int(h[i:i+2], 16) for i in (0, 2, 4))

def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))

def gauge_color(t):
    """t∈[0,1] → 沿 5 段渐变取色（绿→深绿→橙→红）。"""
    stops = [hex2rgb(LV[0]), hex2rgb(LV[1]), hex2rgb(LV[2]), hex2rgb(LV[3])]
    t = max(0.0, min(1.0, t))
    seg = t * (len(stops) - 1)
    i = int(seg)
    if i >= len(stops) - 1:
        return stops[-1]
    return lerp(stops[i], stops[i + 1], seg - i)

def draw_gauge(size, colored, supersample=4):
    """绘制仪表盘。colored=True 为 App 图标，False 为菜单栏模板。"""
    S = size * supersample
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    cx = cy = S / 2

    if colored:
        # 圆角深底
        r = int(S * 0.21)
        d.rounded_rectangle([S*0.03, S*0.03, S*0.97, S*0.97], radius=r, fill=hex2rgb("#1b1b1f") + (255,))

    # 表盘参数：3/4 圆弧，从 135° 到 405°（即底部缺口 90°）
    radius = S * 0.30
    ring_w = int(S * 0.065)
    start_deg, end_deg = 135, 405  # PIL 角度：0°=东，顺时针
    steps = 240
    for i in range(steps):
        t0 = i / steps
        t1 = (i + 1) / steps
        a0 = math.radians(start_deg + (end_deg - start_deg) * t0)
        a1 = math.radians(start_deg + (end_deg - start_deg) * t1)
        # PIL arc 的 0° 在 x 轴正向、顺时针；表盘缺口朝下。
        # t0 从 0→1 = 从左端(绿) → 右端(红)，低用量在左、高用量在右。
        color = gauge_color(t0) if colored else (0, 0, 0)
        bbox = [cx - radius, cy - radius, cx + radius, cy + radius]
        d.arc(bbox, math.degrees(a0), math.degrees(a1), fill=color + (255,), width=ring_w)

    # 圆环两端圆头（消掉直切角）：start 端=绿(t=0)，end 端=红(t=1)
    def cap(deg):
        a = math.radians(deg)
        x, y = cx + radius * math.cos(a), cy + radius * math.sin(a)
        col = (gauge_color(0.0) if deg == start_deg else gauge_color(1.0)) if colored else (0, 0, 0)
        r = ring_w / 2
        d.ellipse([x - r, y - r, x + r, y + r], fill=col + (255,))
    cap(start_deg)
    cap(end_deg)

    # 指针（指向 ~62% 处，橙区）
    ang_deg = start_deg + (end_deg - start_deg) * 0.62
    ang = math.radians(ang_deg)
    plen = radius * 0.92
    px, py = cx + plen * math.cos(ang), cy + plen * math.sin(ang)
    needle_col = (242, 242, 242) if colored else (0, 0, 0)
    d.line([cx, cy, px, py], fill=needle_col + (255,), width=int(S * 0.045))
    # 轴心
    hub_r = S * 0.05
    d.ellipse([cx - hub_r, cy - hub_r, cx + hub_r, cy + hub_r], fill=needle_col + (255,))
    inner = hex2rgb("#1b1b1f") if colored else (0, 0, 0)
    d.ellipse([cx - hub_r*0.45, cy - hub_r*0.45, cx + hub_r*0.45, cy + hub_r*0.45], fill=inner + (255,))

    return img.resize((size, size), Image.LANCZOS)

# App 图标多尺寸
for size in [32, 128, 256, 512, 1024]:
    draw_gauge(size, colored=True).save(f"{OUT}/{size}x{size}.png")
    print(f"app {size}x{size} ✅")
draw_gauge(512, colored=True).save(f"{OUT}/icon.png")

# 菜单栏模板图标（纯黑 + 透明底）
menubar = draw_gauge(512, colored=False)
menubar.save(f"{OUT}/menubar.png")
# 同时导出 RGBA 原始字节（Tauri Image::new_owned 需要 RGBA + 宽高，不直接吃 PNG）
mb_rgba = draw_gauge(64, colored=False)  # 菜单栏实际显示尺寸小，64 够清晰
with open(f"{OUT}/menubar.rgba", "wb") as f:
    f.write(mb_rgba.tobytes())
print("menubar template ✅ (+ menubar.rgba 64x64)")
print("完成")
