//! Which pages need OCR.
//!
//! A page is a **scan** when it has (almost) no text layer and placed images
//! cover most of it: the text is in the pixels. Pages that already carry an
//! OCR text layer (often invisible, over the scan) are **text**: their layer
//! is used as is. Everything else is text too; the user can force OCR on any
//! page from the interface.

use crate::model::PageInput;
use crate::text::visible_len;
use serde::Serialize;

/// Below this many visible characters the text layer does not count.
pub const MIN_TEXT_CHARS: usize = 40;
/// Share of the page that images must cover for a scan.
pub const MIN_IMAGE_COVER: f32 = 0.5;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageKind {
    pub page_num: u32,
    /// `text` | `scan`
    pub kind: String,
    /// Visible characters of the PDF text layer.
    pub chars: usize,
    /// Share of the page covered by images (0–1).
    pub image_cover: f32,
    pub reason: String,
}

/// Share of the page covered by the union of the images, sampled on a grid.
pub fn image_cover(page: &PageInput) -> f32 {
    let pb = page.page_bounds;
    if page.images.is_empty() || pb.w <= 0.0 || pb.h <= 0.0 {
        return 0.0;
    }
    const N: usize = 48;
    let mut hit = 0usize;
    for j in 0..N {
        let y = pb.y + pb.h * (j as f32 + 0.5) / N as f32;
        for i in 0..N {
            let x = pb.x + pb.w * (i as f32 + 0.5) / N as f32;
            if page.images.iter().any(|b| b.contains_point(x, y)) {
                hit += 1;
            }
        }
    }
    hit as f32 / (N * N) as f32
}

pub fn classify_page(page: &PageInput) -> PageKind {
    let chars: usize = page.items.iter().map(|it| visible_len(&it.text)).sum();
    let cover = image_cover(page);
    let (kind, reason) = if chars < MIN_TEXT_CHARS && cover >= MIN_IMAGE_COVER {
        ("scan", format!("{chars} caratteri nello strato di testo, immagini sul {:.0}% della pagina", cover * 100.0))
    } else if chars >= MIN_TEXT_CHARS {
        ("text", format!("{chars} caratteri nello strato di testo"))
    } else {
        ("text", format!("{chars} caratteri, immagini sul {:.0}% della pagina: nessun indizio di scansione", cover * 100.0))
    };
    PageKind { page_num: page.page_num, kind: kind.into(), chars, image_cover: cover, reason }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Bounds, TextElement};

    fn page(items: Vec<TextElement>, images: Vec<Bounds>) -> PageInput {
        PageInput { page_num: 1, page_bounds: Bounds { x: 0.0, y: 0.0, w: 600.0, h: 800.0 }, items, bordered_boxes: vec![], images }
    }
    fn run(text: &str) -> TextElement {
        TextElement { text: text.into(), x: 10.0, y: 10.0, width: 100.0, height: 10.0, font_size: 10.0, ..Default::default() }
    }

    #[test]
    fn full_page_image_without_text_is_a_scan() {
        let p = page(vec![run("12")], vec![Bounds { x: 0.0, y: 0.0, w: 600.0, h: 800.0 }]);
        assert_eq!(classify_page(&p).kind, "scan");
    }

    #[test]
    fn scan_with_a_text_layer_keeps_its_layer() {
        let items = (0..20).map(|_| run("hidden OCR text layer")).collect();
        let p = page(items, vec![Bounds { x: 0.0, y: 0.0, w: 600.0, h: 800.0 }]);
        assert_eq!(classify_page(&p).kind, "text");
    }

    #[test]
    fn strips_of_images_add_up() {
        let strips = (0..4).map(|k| Bounds { x: 0.0, y: 200.0 * k as f32, w: 600.0, h: 200.0 }).collect();
        let p = page(vec![], strips);
        let k = classify_page(&p);
        assert_eq!(k.kind, "scan");
        assert!(k.image_cover > 0.95);
    }

    #[test]
    fn small_figure_on_an_empty_page_is_not_a_scan() {
        let p = page(vec![], vec![Bounds { x: 100.0, y: 100.0, w: 200.0, h: 200.0 }]);
        assert_eq!(classify_page(&p).kind, "text");
    }
}
