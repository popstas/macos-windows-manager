#!/usr/bin/env python3
"""Значок трея: белые буквы WM на прозрачном фоне.

Руками `src-tauri/icons/icon.png` не правится — он вывод этого скрипта, и
правка переживёт ровно до следующего запуска. То же правило, что у соседнего
`ccfzf-picker/scripts/make-icons.py`.

Размер взят с запасом под retina: строка меню macOS рисует значок высотой
около 22 точек, а на экране с двойной плотностью это 44 пикселя. Меньший
исходник дал бы мыло.
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

SIZE = 128
TEXT = "WM"
OUT = Path(__file__).resolve().parent.parent / "src-tauri" / "icons" / "icon.png"

# Шрифты, которые встречаются на машинах сборки. Первый найденный и берём:
# конкретное начертание тут не принципиально, важна жирность — тонкие штрихи
# в строке меню съедаются сглаживанием.
FONTS = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/truetype/freefont/FreeSansBold.ttf",
    "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
]


def pick_font(size: int) -> ImageFont.FreeTypeFont:
    for path in FONTS:
        if Path(path).exists():
            return ImageFont.truetype(path, size)
    # Запасной вариант рисует мелко и без жирности, но собраться позволяет.
    return ImageFont.load_default()


def main() -> None:
    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # Кегль подбирается под ширину, а не задаётся числом: "WM" — две широкие
    # буквы, и заданный на глаз кегль вылезал бы за края при смене шрифта.
    size = SIZE
    while size > 8:
        font = pick_font(size)
        box = draw.textbbox((0, 0), TEXT, font=font)
        if box[2] - box[0] <= SIZE * 0.92 and box[3] - box[1] <= SIZE * 0.72:
            break
        size -= 2

    font = pick_font(size)
    box = draw.textbbox((0, 0), TEXT, font=font)
    x = (SIZE - (box[2] - box[0])) / 2 - box[0]
    y = (SIZE - (box[3] - box[1])) / 2 - box[1]
    draw.text((x, y), TEXT, font=font, fill=(255, 255, 255, 255))

    OUT.parent.mkdir(parents=True, exist_ok=True)
    img.save(OUT)
    print(f"{OUT} {SIZE}x{SIZE} кегль {size}")


if __name__ == "__main__":
    main()
