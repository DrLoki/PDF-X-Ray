//! Page layout of a comic page, built from its [`ComicLayout`] instead of
//! XY-Cut.
//!
//! Text runs are first split by container (the smallest one holding the
//! run's centre) and lines are built inside each container separately, so
//! two balloons side by side never share a line. Each container becomes
//! one leaf (one translation unit); the tree is root → panel → balloon /
//! narration box → leaf, with free text and sound effects directly in the
//! panel. Runs outside every container become one leaf per line.

use super::order::{assign_panels, balloon_order, panel_order};
use super::{ComicLayout, KIND_BALLOON, KIND_NARRATION, KIND_SFX, KIND_TEXT};
use crate::lines::{build_lines, group_rows, Line, PageLines};
use crate::model::{Bounds, LayoutNode, PageInput, Projections, TextElement};
use crate::xycut::{leaf_formatting, Leaf, PageLayout};

pub struct ComicPagePlan {
    pub lines: PageLines,
    pub layout: PageLayout,
    /// Role of each leaf of `layout.leaves` (the container kind).
    pub roles: Vec<&'static str>,
}

fn role_of(kind: &str) -> &'static str {
    match kind {
        KIND_BALLOON => KIND_BALLOON,
        KIND_NARRATION => KIND_NARRATION,
        KIND_SFX => KIND_SFX,
        _ => KIND_TEXT,
    }
}

struct Container {
    bounds: Bounds,
    kind: &'static str,
    /// Line indices (into the page lines).
    lines: Vec<usize>,
}

fn node(id: String, node_type: &str, bounds: Bounds, depth: usize) -> LayoutNode {
    LayoutNode {
        id,
        node_type: node_type.into(),
        bounds,
        depth,
        cut_direction: None,
        is_bordered_box: None,
        is_table: None,
        text: None,
        formatting: None,
        role: None,
        reading_order: None,
        paragraph_ids: None,
        children: None,
        diag: None,
        container_kind: None,
    }
}

pub fn plan_comic_page(page: &PageInput, comic: &ComicLayout, rtl: bool) -> ComicPagePlan {
    let grow = |b: &Bounds| Bounds { x: b.x - 1.5, y: b.y - 1.5, w: b.w + 3.0, h: b.h + 3.0 };
    // ---- runs → containers
    let mut buckets: Vec<Vec<TextElement>> = vec![Vec::new(); comic.balloons.len()];
    let mut orphans: Vec<TextElement> = Vec::new();
    for it in &page.items {
        let (cx, cy) = (it.x + it.width * 0.5, it.y + it.height * 0.5);
        let owner = comic
            .balloons
            .iter()
            .enumerate()
            .filter(|(_, b)| grow(&b.bounds).contains_point(cx, cy))
            .min_by(|a, b| a.1.bounds.area().total_cmp(&b.1.bounds.area()))
            .map(|(k, _)| k);
        match owner {
            Some(k) => buckets[k].push(it.clone()),
            None => orphans.push(it.clone()),
        }
    }
    // ---- lines, built per container
    let mut lines: Vec<Line> = Vec::new();
    let mut containers: Vec<Container> = Vec::new();
    for (k, items) in buckets.iter().enumerate() {
        if items.is_empty() {
            continue;
        }
        let pl = build_lines(items);
        let start = lines.len();
        lines.extend(pl.lines);
        let b = &comic.balloons[k];
        containers.push(Container { bounds: b.bounds, kind: role_of(&b.kind), lines: (start..lines.len()).collect() });
    }
    if !orphans.is_empty() {
        // stray marks read as text outside every container are dropped
        // (the detection drops them too: see `balloons`)
        for l in build_lines(&orphans).lines.into_iter().filter(|l| l.text.chars().filter(|c| c.is_alphanumeric()).count() >= 2) {
            containers.push(Container { bounds: l.b, kind: KIND_TEXT, lines: vec![lines.len()] });
            lines.push(l);
        }
    }

    // ---- order: panels (and containers outside every panel) as slots
    let cbounds: Vec<Bounds> = containers.iter().map(|c| c.bounds).collect();
    let owner = assign_panels(&comic.panels, &cbounds);
    #[derive(Clone, Copy)]
    enum Slot {
        Panel(usize),
        Loose(usize),
    }
    let mut slots: Vec<Slot> = (0..comic.panels.len()).map(Slot::Panel).collect();
    slots.extend((0..containers.len()).filter(|&c| owner[c].is_none()).map(Slot::Loose));
    let slot_bounds: Vec<Bounds> = slots
        .iter()
        .map(|s| match *s {
            Slot::Panel(p) => comic.panels[p],
            Slot::Loose(c) => containers[c].bounds,
        })
        .collect();
    let tol = page.page_bounds.h * 0.01;
    let slot_order = panel_order(&slot_bounds, rtl, tol);

    // ---- tree
    let pn = page.page_num;
    let mut leaves: Vec<Leaf> = Vec::new();
    let mut roles: Vec<&'static str> = Vec::new();
    let mut counter = 0usize;
    let mut make_leaf = |c: &Container, depth: usize, leaves: &mut Vec<Leaf>, roles: &mut Vec<&'static str>| -> LayoutNode {
        counter += 1;
        let id = format!("p{pn}-b{counter}");
        let rows = group_rows(&lines, &c.lines);
        let ordered: Vec<usize> = rows.iter().flatten().copied().collect();
        let tb = Bounds::union_all(c.lines.iter().map(|&i| &lines[i].b)).unwrap_or(c.bounds);
        let text = rows.iter().map(|r| r.iter().map(|&i| lines[i].text.as_str()).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n");
        let formatting = leaf_formatting(&lines, &rows, &tb);
        leaves.push(Leaf { id: id.clone(), lines: ordered, bounds: tb, box_kind: None, box_id: None });
        roles.push(c.kind);
        LayoutNode { text: Some(text), formatting: Some(formatting), ..node(id, "leaf", tb, depth) }
    };
    let wrap = |c: &Container, depth: usize, leaf: LayoutNode, idx: usize| -> LayoutNode {
        if c.kind == KIND_BALLOON || c.kind == KIND_NARRATION {
            LayoutNode {
                is_bordered_box: Some(true),
                children: Some(vec![LayoutNode { depth: depth + 1, ..leaf }]),
                container_kind: Some(c.kind.to_string()),
                ..node(format!("p{pn}-c{idx}"), "container", c.bounds, depth)
            }
        } else {
            leaf
        }
    };
    let mut root_children: Vec<LayoutNode> = Vec::new();
    for &s in &slot_order {
        match slots[s] {
            Slot::Panel(p) => {
                let members: Vec<usize> = (0..containers.len()).filter(|&c| owner[c] == Some(p)).collect();
                let order = balloon_order(&members, &cbounds, rtl);
                let mut kids = Vec::new();
                for c in order {
                    let boxed = containers[c].kind == KIND_BALLOON || containers[c].kind == KIND_NARRATION;
                    let leaf = make_leaf(&containers[c], if boxed { 3 } else { 2 }, &mut leaves, &mut roles);
                    kids.push(wrap(&containers[c], 2, leaf, c + 1));
                }
                root_children.push(LayoutNode {
                    children: Some(kids),
                    container_kind: Some("panel".into()),
                    ..node(format!("p{pn}-panel{}", p + 1), "container", comic.panels[p], 1)
                });
            }
            Slot::Loose(c) => {
                let boxed = containers[c].kind == KIND_BALLOON || containers[c].kind == KIND_NARRATION;
                let leaf = make_leaf(&containers[c], if boxed { 2 } else { 1 }, &mut leaves, &mut roles);
                root_children.push(wrap(&containers[c], 1, leaf, c + 1));
            }
        }
    }
    let root = LayoutNode {
        children: Some(root_children),
        container_kind: Some("comic".into()),
        ..node(format!("p{pn}-root"), "root", page.page_bounds, 0)
    };
    let projections = Projections { x_gaps: Vec::new(), y_gaps: Vec::new(), page_bounds: page.page_bounds };
    ComicPagePlan { lines: PageLines { lines, drop_caps: Vec::new() }, layout: PageLayout { root, leaves, projections }, roles }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comic::ComicBalloon;

    fn run(text: &str, x: f32, y: f32, w: f32) -> TextElement {
        TextElement { text: text.into(), x, y, width: w, height: 10.0, font_size: 8.0, font_name: Some("OCR".into()), ..Default::default() }
    }
    fn b(x: f32, y: f32, w: f32, h: f32) -> Bounds {
        Bounds { x, y, w, h }
    }

    #[test]
    fn one_leaf_per_container_in_comic_order() {
        // two panels side by side; in the left one two balloons whose lines
        // sit on the same rows (they must not merge), in the right one a
        // narration box
        let page = PageInput {
            page_num: 3,
            page_bounds: b(0.0, 0.0, 400.0, 300.0),
            items: vec![
                run("LEFT ONE", 20.0, 20.0, 60.0),
                run("SPEAKS", 22.0, 31.0, 50.0),
                run("RIGHT", 100.0, 20.0, 40.0),
                run("ANSWERS", 98.0, 31.0, 50.0),
                run("MEANWHILE...", 220.0, 20.0, 90.0),
                run("17", 380.0, 285.0, 10.0),
            ],
            bordered_boxes: vec![],
            images: vec![],
            comic: None,
        };
        let comic = ComicLayout {
            panels: vec![b(10.0, 10.0, 180.0, 260.0), b(200.0, 10.0, 180.0, 260.0)],
            balloons: vec![
                ComicBalloon { bounds: b(95.0, 15.0, 60.0, 30.0), kind: KIND_BALLOON.into(), lines: vec![] },
                ComicBalloon { bounds: b(15.0, 15.0, 70.0, 30.0), kind: KIND_BALLOON.into(), lines: vec![] },
                ComicBalloon { bounds: b(210.0, 15.0, 110.0, 20.0), kind: KIND_NARRATION.into(), lines: vec![] },
            ],
            is_comic: true,
        };
        let plan = plan_comic_page(&page, &comic, false);
        let texts: Vec<String> = plan.layout.leaves.iter().map(|l| l.lines.iter().map(|&i| plan.lines.lines[i].text.clone()).collect::<Vec<_>>().join(" ")).collect();
        assert_eq!(texts, vec!["LEFT ONE SPEAKS", "RIGHT ANSWERS", "MEANWHILE...", "17"]);
        assert_eq!(plan.roles, vec![KIND_BALLOON, KIND_BALLOON, KIND_NARRATION, KIND_TEXT]);
        let rtl = plan_comic_page(&page, &comic, true);
        let first = &rtl.layout.leaves[0];
        assert_eq!(rtl.lines.lines[first.lines[0]].text, "MEANWHILE...");
    }
}
