#!/usr/bin/env python3
"""从 design/src/icons.svg 拆出每个 <symbol> 为独立 SVG 文件，供 nsh asset!() 使用。

- currentColor -> 字面色（tint_color 运行时重着色；i-cursor 保留白色描边，用时不上 tint）
- 输出: gui/assets/icons/<name>.svg（i-send -> send.svg）
用法: python tools/split_icons.py
"""

import re
import sys
import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "design" / "src" / "icons.svg"
OUT = ROOT / "gui" / "assets" / "icons"

INK = "#202331"      # i-cursor 的实心指针底色
ICON = "#5F6678"     # 其余图标的占位描边色（被 tint 覆盖）

SYMBOL_RE = re.compile(r'<symbol id="i-([a-z-]+)" viewBox="([^"]+)">(.*?)</symbol>', re.S)


def main() -> None:
    text = SRC.read_text(encoding="utf-8")
    OUT.mkdir(parents=True, exist_ok=True)
    count = 0
    for m in SYMBOL_RE.finditer(text):
        name, viewBox, body = m.group(1), m.group(2), m.group(3)
        color = INK if name == "cursor" else ICON
        body = body.replace("currentColor", color)
        svg = (
            '<svg xmlns="http://www.w3.org/2000/svg" viewBox="%s" '
            'width="24" height="24" fill="none">%s</svg>\n' % (viewBox, body)
        )
        (OUT / f"{name}.svg").write_text(svg, encoding="utf-8")
        count += 1
    print(f"wrote {count} icons to {OUT}")
    if count != 26:
        sys.exit(f"expected 26 symbols, found {count}")


if __name__ == "__main__":
    main()
