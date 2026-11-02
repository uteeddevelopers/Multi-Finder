#!/usr/bin/env python3
"""Multi-Finder 앱 아이콘(자체 디자인) 생성.

세 개의 폴더가 한 화면에 겹쳐 펼쳐진 모습 — 앱의 핵심(여러 폴더를 한 번에 본다)을 형태로 옮긴 것.
macOS 앱 아이콘 규격: 1024px 정사각, 모서리 둥근 사각형(약 22.4% 반경), 투명 배경.
출력: AppIcon.iconset/ (iconutil -c icns 로 .icns 생성)
"""
from PIL import Image, ImageDraw, ImageFilter
import os, sys

S = 1024
out_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "AppIcon.iconset")


def rounded_rect(draw, box, r, fill):
    draw.rounded_rectangle(box, radius=r, fill=fill)


def folder(w, h, body, tab, shade):
    """폴더 한 장을 RGBA 이미지로. 탭(왼쪽 위 돌출) + 본체 + 아래쪽 약간 어두운 띠."""
    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    tab_w, tab_h = int(w * 0.38), int(h * 0.16)
    r = int(min(w, h) * 0.09)
    # 탭
    d.rounded_rectangle((0, 0, tab_w, tab_h * 2), radius=r, fill=tab)
    # 본체
    d.rounded_rectangle((0, tab_h, w, h), radius=r, fill=body)
    # 본체 윗부분 밝은 띠 (종이가 살짝 보이는 느낌)
    d.rounded_rectangle((int(w * 0.04), tab_h + int(h * 0.05), w - int(w * 0.04), tab_h + int(h * 0.11)), radius=r // 2, fill=shade)
    return img


def main():
    base = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    # 배경: 둥근 사각형 + 위아래 그라데이션(파랑)
    bg = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    grad = Image.new("RGBA", (S, S))
    gd = ImageDraw.Draw(grad)
    top, bottom = (56, 132, 255), (18, 78, 196)
    for y in range(S):
        t = y / (S - 1)
        c = tuple(int(top[i] * (1 - t) + bottom[i] * t) for i in range(3)) + (255,)
        gd.line([(0, y), (S, y)], fill=c)
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, S - 1, S - 1), radius=int(S * 0.224), fill=255)
    bg.paste(grad, (0, 0), mask)

    # 폴더 세 장: 뒤 → 앞으로 갈수록 크고 밝게, 살짝 어긋나게 쌓음
    layers = [
        (0.50, 0.36, (168, 200, 255, 230), (150, 186, 250, 230), (200, 220, 255, 255), (0.16, 0.19)),
        (0.52, 0.37, (214, 230, 255, 245), (196, 216, 252, 245), (236, 244, 255, 255), (0.24, 0.31)),
        (0.54, 0.38, (255, 255, 255, 255), (238, 243, 252, 255), (255, 255, 255, 255), (0.32, 0.44)),
    ]
    for fw, fh, body, tab, shade, (ox, oy) in layers:
        w, h = int(S * fw), int(S * fh)
        f = folder(w, h, body, tab, shade)
        # 그림자
        shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
        sh = Image.new("RGBA", (w, h), (0, 0, 0, 0))
        ImageDraw.Draw(sh).rounded_rectangle((0, int(h * 0.16), w, h), radius=int(min(w, h) * 0.09), fill=(0, 20, 80, 90))
        shadow.paste(sh, (int(S * ox), int(S * oy) + int(S * 0.02)))
        shadow = shadow.filter(ImageFilter.GaussianBlur(S * 0.018))
        bg = Image.alpha_composite(bg, shadow)
        layer = Image.new("RGBA", (S, S), (0, 0, 0, 0))
        layer.paste(f, (int(S * ox), int(S * oy)))
        bg = Image.alpha_composite(bg, layer)

    # 모서리 밖으로 나간 부분은 잘라낸다
    clipped = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    clipped.paste(bg, (0, 0), mask)
    base = Image.alpha_composite(base, clipped)
    os.makedirs(out_dir, exist_ok=True)
    names = {16: ["icon_16x16.png"], 32: ["icon_16x16@2x.png", "icon_32x32.png"], 64: ["icon_32x32@2x.png"],
             128: ["icon_128x128.png"], 256: ["icon_128x128@2x.png", "icon_256x256.png"],
             512: ["icon_256x256@2x.png", "icon_512x512.png"], 1024: ["icon_512x512@2x.png"]}
    for s, files in names.items():
        im = base.resize((s, s), Image.LANCZOS)
        for f in files:
            im.save(os.path.join(out_dir, f))
    base.save(os.path.join(os.path.dirname(out_dir), "AppIcon-preview.png"))
    print("→", out_dir)


if __name__ == "__main__":
    main()
