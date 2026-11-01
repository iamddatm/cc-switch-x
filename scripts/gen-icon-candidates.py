#!/usr/bin/env python
"""图标候选生成器：按可调参数画 app 图标，出多组配色与 16/32px 缩略对照。

用法（仓库根目录）：
    uv run --with pillow scripts/gen-icon-candidates.py
    uv run --with pillow scripts/gen-icon-candidates.py --spokes 6 --palettes A,D
    uv run --with pillow scripts/gen-icon-candidates.py --size 1024

产出在 icon-lab/（已 gitignore）：每组配色一张 master PNG，外加一张 contact sheet。
master 确认后交给 `pnpm tauri icon icon-lab/master-A.png` 生成全套 PNG / .ico / .icns，
不要手改 src-tauri/icons/。
"""

from __future__ import annotations

import argparse
import math
import sys
from pathlib import Path

# Git Bash 下 Python 默认按 GBK 写管道，中文输出会乱码。
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")

from PIL import Image, ImageDraw, ImageFont

# 配色候选：tile 底、spokes 两端色、dot 签名色（= 应用内 --action）。
# 主条一律走冷色，与上游 cc-switch 的橙主色拉开 150° 以上色相；橙只留在中心点。
PALETTES: dict[str, dict] = {
    "A": {
        "name": "青蓝→靛紫 / 墨蓝底（主推）",
        "tile": "#14152B",
        "spokes": ("#22A7C0", "#3B5BDB"),
        "dot": "#F97316",
    },
    "B": {
        "name": "青绿→青蓝 / 深墨底（小尺寸对比更强）",
        "tile": "#0B1020",
        "spokes": ("#2BB673", "#38BDF8"),
        "dot": "#F97316",
    },
    "C": {
        "name": "青蓝→靛紫 / 无底（贴近上游透明底做法）",
        "tile": None,
        "spokes": ("#22A7C0", "#3B5BDB"),
        "dot": "#F97316",
    },
    "D": {
        "name": "冷色条 / 浅色底（浅色桌面版）",
        "tile": "#F5F6FA",
        "spokes": ("#0E7490", "#3730A3"),
        "dot": "#EA580C",
    },
}

# 几何一律按画布边长的比例给，换尺寸不重画。
GEOMETRY = {
    "inset": 0.045,        # 圆角方底留白
    "corner": 0.225,       # 底块圆角半径 / 边长
    "spoke_inner": 0.124,  # 放射条内端半径：内缩到明显压在中心点下方，避免缩图时露缝
    "spoke_outer": 0.340,  # 放射条外端半径
    "spoke_width": 0.075,  # 放射条粗细
    "dot_radius": 0.104,   # 中心签名点半径
}


def rgb(hex_color: str) -> tuple[int, int, int]:
    value = hex_color.lstrip("#")
    return tuple(int(value[i : i + 2], 16) for i in (0, 2, 4))  # type: ignore[return-value]


def mix(a: tuple[int, int, int], b: tuple[int, int, int], t: float) -> tuple[int, int, int]:
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))  # type: ignore[return-value]


def luminance(color: tuple[int, int, int]) -> float:
    def channel(value: int) -> float:
        v = value / 255.0
        return v / 12.92 if v <= 0.03928 else ((v + 0.055) / 1.055) ** 2.4

    r, g, b = (channel(value) for value in color)
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def contrast(a: str, b: str) -> float:
    la, lb = luminance(rgb(a)), luminance(rgb(b))
    light, dark = max(la, lb), min(la, lb)
    return (light + 0.05) / (dark + 0.05)


def spoke_color(palette: dict, index: int, total: int) -> tuple[int, int, int]:
    """沿环对称地插值：转一圈回到起点，避免接缝处出现色跳。"""
    start, end = (rgb(color) for color in palette["spokes"])
    phase = (index % total) / total
    t = 1.0 - abs(phase * 2.0 - 1.0)
    return mix(start, end, t)


def capsule(
    draw: ImageDraw.ImageDraw,
    p0: tuple[float, float],
    p1: tuple[float, float],
    width: float,
    color: tuple[int, int, int],
) -> None:
    """圆头线段：直线段 + 两端整圆，缩到 16px 时端点不会变成方角。"""
    draw.line([p0, p1], fill=color, width=round(width))
    radius = width / 2.0
    for point in (p0, p1):
        draw.ellipse(
            [point[0] - radius, point[1] - radius, point[0] + radius, point[1] + radius],
            fill=color,
        )


def render(
    palette: dict,
    size: int,
    spokes: int,
    supersample: int,
    tilt: float,
    geometry: dict | None = None,
) -> Image.Image:
    # 用调用方的覆盖值补掉 GEOMETRY 的同名键，换辐条数时不必改默认值
    geo = {**GEOMETRY, **(geometry or {})}
    side = size * supersample
    canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    draw = ImageDraw.Draw(canvas)

    if palette["tile"]:
        margin = side * geo["inset"]
        draw.rounded_rectangle(
            [margin, margin, side - margin, side - margin],
            radius=side * (1.0 - 2 * geo["inset"]) * geo["corner"],
            fill=rgb(palette["tile"]) + (255,),
        )

    center = side / 2.0
    r_in = side * geo["spoke_inner"]
    r_out = side * geo["spoke_outer"]
    width = side * geo["spoke_width"]

    for index in range(spokes):
        angle = math.radians(-90.0 + tilt + index * 360.0 / spokes)
        direction = (math.cos(angle), math.sin(angle))
        start = (center + direction[0] * r_in, center + direction[1] * r_in)
        stop = (center + direction[0] * r_out, center + direction[1] * r_out)
        capsule(draw, start, stop, width, spoke_color(palette, index, spokes))

    dot = side * geo["dot_radius"]
    draw.ellipse(
        [center - dot, center - dot, center + dot, center + dot],
        fill=rgb(palette["dot"]) + (255,),
    )

    return canvas.resize((size, size), Image.LANCZOS)


def contact_sheet(samples: list[tuple[str, dict, Image.Image]], sizes: list[int]) -> Image.Image:
    """把每组成品分别贴在浅色与深色桌面上按原尺寸排一遍，小尺寸糊不糊只能这样看。"""
    pad = 18
    label_h = 22
    column = max(sizes) + pad  # 列宽按最大缩略图，行高按各自尺寸排，免得留大片空白
    rows = [(size, tone) for size in sizes for tone in (rgb("#F5F5F7"), rgb("#1C1C1E"))]
    width = pad + column * len(samples)
    height = label_h + pad + sum(size + pad for size, _ in rows)
    sheet = Image.new("RGBA", (width, height), (255, 255, 255, 255))
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.load_default()

    for index, (key, palette, _) in enumerate(samples):
        ratio = contrast(palette["spokes"][0], palette["tile"] or "#FFFFFF")
        draw.text(
            (pad + index * column, 4),
            f"{key}  {ratio:.2f}:1",
            fill=(20, 20, 25, 255),
            font=font,
        )

    y = label_h + pad
    for size, background in rows:
        draw.rectangle([0, y, width, y + size + pad], fill=background + (255,))
        draw.text((4, y + 2), f"{size}px", fill=(140, 140, 150, 255), font=font)
        for index, (_, _, master) in enumerate(samples):
            thumb = master.resize((size, size), Image.LANCZOS)
            sheet.alpha_composite(thumb, (pad + index * column, y + 2))
        y += size + pad
    return sheet


def main() -> None:
    parser = argparse.ArgumentParser(description="生成 app 图标配色候选")
    parser.add_argument("--size", type=int, default=512, help="master 边长，默认 512")
    parser.add_argument("--spokes", type=int, default=8, help="放射条数量，默认 8（上游是 12）")
    parser.add_argument("--tilt", type=float, default=0.0, help="整体旋转角度，用于错开竖直条")
    parser.add_argument("--supersample", type=int, default=4, help="超采样倍数，抗锯齿用")
    parser.add_argument("--palettes", default=",".join(PALETTES), help="要出的配色键，逗号分隔")
    parser.add_argument("--spoke-width", type=float, help=f"放射条粗细 / 边长，默认 {GEOMETRY['spoke_width']}")
    parser.add_argument("--spoke-outer", type=float, help=f"放射条外端半径 / 边长，默认 {GEOMETRY['spoke_outer']}")
    parser.add_argument("--dot-radius", type=float, help=f"中心点半径 / 边长，默认 {GEOMETRY['dot_radius']}")
    parser.add_argument("--no-tile", action="store_true", help="去掉圆角方底，图标只留放射条与中心点（透明底）")
    parser.add_argument("--thumbs", default="256,128,64,32,16", help="缩略对照尺寸列表")
    parser.add_argument("--out", default="icon-lab", help="输出目录")
    args = parser.parse_args()

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    keys = [key.strip().upper() for key in args.palettes.split(",") if key.strip()]
    samples: list[tuple[str, dict, Image.Image]] = []
    geometry = {
        key: value
        for key, value in (
            ("spoke_width", args.spoke_width),
            ("spoke_outer", args.spoke_outer),
            ("dot_radius", args.dot_radius),
        )
        if value is not None
    }

    for key in keys:
        palette = PALETTES[key]
        if args.no_tile:
            palette = {**palette, "tile": None}
        master = render(palette, args.size, args.spokes, args.supersample, args.tilt, geometry)
        path = out / f"master-{key}.png"
        master.save(path)
        samples.append((key, palette, master))
        tile = palette["tile"] or "transparent"
        print(
            f"{key} {palette['name']:<28} 底={tile:<9} "
            f"条/底对比 {contrast(palette['spokes'][0], palette['tile'] or '#FFFFFF'):.2f}:1  "
            f"点/底对比 {contrast(palette['dot'], palette['tile'] or '#FFFFFF'):.2f}:1  -> {path}"
        )

    sizes = [int(value) for value in args.thumbs.split(",") if value.strip()]
    sheet = contact_sheet(samples, sizes)
    sheet_path = out / "contact-sheet.png"
    sheet.save(sheet_path)
    print(f"缩略对照（浅/深桌面 × {'/'.join(str(s) for s in sizes)}px）：{sheet_path}")
    print(f"选定后执行：pnpm tauri icon {out / f'master-{keys[0]}.png'}")


if __name__ == "__main__":
    main()
