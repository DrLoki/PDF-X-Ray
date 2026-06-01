#!/usr/bin/env python3
import sys
import json

try:
    import fitz  # PyMuPDF
except Exception as e:
    print('MISSING_PYMUPDF', file=sys.stderr)
    raise

if len(sys.argv) < 3:
    print('Usage: python tools/extract_page_py.py <pdfPath> <pageNum>', file=sys.stderr)
    sys.exit(1)

pdf_path = sys.argv[1]
page_num = int(sys.argv[2])

doc = fitz.open(pdf_path)
if page_num < 1 or page_num > doc.page_count:
    print('Invalid page number', file=sys.stderr)
    sys.exit(2)

page = doc.load_page(page_num - 1)
# Use default zoom = 1.0
rect = page.rect
page_w = rect.width
page_h = rect.height

text_json = page.get_text('dict')
items = []
for block in text_json.get('blocks', []):
    if block.get('type') != 0:
        continue
    for line in block.get('lines', []):
        for span in line.get('spans', []):
            text = span.get('text', '')
            if not text.strip():
                continue
            bbox = span.get('bbox', [0,0,0,0])
            x0, y0, x1, y1 = bbox
            width = x1 - x0
            height = y1 - y0
            size = span.get('size', 0)
            font = span.get('font', None)
            items.append({
                'text': text,
                'x': x0,
                'y': y0,
                'width': width,
                'height': height,
                'font_size': size,
                'font_name': font
            })

out = {
    'items': items,
    'pageBounds': {'x': 0, 'y': 0, 'w': page_w, 'h': page_h},
    'borderedBoxes': []
}
print(json.dumps(out))
