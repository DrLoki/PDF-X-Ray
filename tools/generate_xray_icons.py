from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter

OUTPUT_DIR = Path('src-tauri/icons')
OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

SIZES = [32, 128, 256, 512]


def create_xray_icon(size):
    img = Image.new('RGBA', (size, size), (3, 6, 18, 255))
    draw = ImageDraw.Draw(img)

    # Background glow radial gradients
    for i in range(size // 2, 0, -1):
        alpha = int(80 * (1 - i / (size // 2)))
        if alpha <= 0:
            continue
        color = (0, 240, 255, alpha)
        bbox = [size // 2 - i, size // 2 - i, size // 2 + i, size // 2 + i]
        draw.ellipse(bbox, outline=color)

    # Vertical grid lines
    step = max(4, size // 8)
    for x in range(0, size, step):
        line_alpha = 20 if x % (step * 2) == 0 else 10
        draw.line([(x, 0), (x, size)], fill=(0, 240, 255, line_alpha), width=1)
    for y in range(0, size, step):
        line_alpha = 20 if y % (step * 2) == 0 else 10
        draw.line([(0, y), (size, y)], fill=(0, 240, 255, line_alpha), width=1)

    # Central X mark
    line_width = max(12, size // 14)
    x1 = size * 0.2
    y1 = size * 0.2
    x2 = size * 0.8
    y2 = size * 0.8
    draw.line([(x1, y1), (x2, y2)], fill=(165, 255, 255, 255), width=line_width)
    draw.line([(x2, y1), (x1, y2)], fill=(165, 255, 255, 255), width=line_width)

    # Inner cross highlight
    glow = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    gdraw = ImageDraw.Draw(glow)
    glow_width = line_width * 3
    gdraw.line([(x1, y1), (x2, y2)], fill=(0, 240, 255, 120), width=glow_width)
    gdraw.line([(x2, y1), (x1, y2)], fill=(0, 240, 255, 120), width=glow_width)
    glow = glow.filter(ImageFilter.GaussianBlur(radius=size * 0.02))
    img = Image.alpha_composite(img, glow)

    # Outer ring border
    ring_width = max(6, size // 30)
    draw.ellipse([ring_width/2, ring_width/2, size - ring_width/2, size - ring_width/2], outline=(0, 240, 255, 80), width=ring_width)

    return img


def main():
    generated = {}
    for size in SIZES:
        img = create_xray_icon(size)
        path = OUTPUT_DIR / f'{size}x{size}.png'
        img.save(path, format='PNG')
        generated[size] = path
        print(f'Wrote {path}')

    # Save primary icon.png at 256
    main_icon = create_xray_icon(256)
    main_icon.save(OUTPUT_DIR / 'icon.png', format='PNG')
    print('Wrote icon.png')

    # Save ICO with several sizes
    ico_sizes = [(32, 32), (64, 64), (128, 128), (256, 256)]
    main_icon.save(OUTPUT_DIR / 'icon.ico', format='ICO', sizes=ico_sizes)
    print('Wrote icon.ico')

    # Save ICNS if supported
    try:
        main_icon.save(OUTPUT_DIR / 'icon.icns', format='ICNS')
        print('Wrote icon.icns')
    except Exception as e:
        print('Could not write icon.icns:', e)


if __name__ == '__main__':
    main()
