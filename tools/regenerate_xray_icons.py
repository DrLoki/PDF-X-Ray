from pathlib import Path
from PIL import Image

base_path = Path('src-tauri/icons/256x256.png')
base = Image.open(base_path)
for size, filename in [(128, '128x128@2x.png'), (32, '32x32.png')]:
    resized = base.resize((size, size), Image.LANCZOS)
    out_path = Path('src-tauri/icons') / filename
    resized.save(out_path, format='PNG')
    print(f'Updated {out_path}')
