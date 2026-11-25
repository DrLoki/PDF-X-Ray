//! Reading order of a comic page.
//!
//! Panels: recursive cuts on their boxes, like the XY-cut of text blocks.
//! A horizontal cut (no panel crosses it) splits the page into tiers read
//! top to bottom; inside a tier a vertical cut splits it into columns read
//! left to right, or right to left for manga-style books. A tier made of a
//! tall panel next to stacked ones is cut vertically first, so the stacked
//! panels are read one after the other before the next column.
//!
//! Containers inside a panel: repeatedly take the topmost one, but prefer a
//! container on the reading side (left, or right for RTL) whose top is above
//! the middle of the topmost one — the usual "top-left first" rule.

use crate::model::Bounds;

/// Order of `boxes` (indices), tiers then columns. `tol` is the overlap
/// tolerated across a cut (panels drawn slightly overlapping).
pub fn panel_order(boxes: &[Bounds], rtl: bool, tol: f32) -> Vec<usize> {
    let mut out = Vec::with_capacity(boxes.len());
    rec((0..boxes.len()).collect(), boxes, rtl, tol, &mut out, 0);
    out
}

fn split(mut idx: Vec<usize>, start: impl Fn(&Bounds) -> f32, end: impl Fn(&Bounds) -> f32, boxes: &[Bounds], tol: f32) -> Vec<Vec<usize>> {
    idx.sort_by(|&a, &b| start(&boxes[a]).total_cmp(&start(&boxes[b])));
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut reach = f32::NEG_INFINITY;
    for i in idx {
        let b = &boxes[i];
        if groups.is_empty() || start(b) < reach - tol {
            if groups.is_empty() {
                groups.push(Vec::new());
            }
        } else {
            groups.push(Vec::new());
        }
        groups.last_mut().unwrap().push(i);
        reach = reach.max(end(b));
    }
    groups
}

fn rec(idx: Vec<usize>, boxes: &[Bounds], rtl: bool, tol: f32, out: &mut Vec<usize>, depth: usize) {
    if idx.len() <= 1 || depth > 16 {
        out.extend(idx);
        return;
    }
    let tiers = split(idx.clone(), |b| b.y, |b| b.y1(), boxes, tol);
    if tiers.len() > 1 {
        for t in tiers {
            rec(t, boxes, rtl, tol, out, depth + 1);
        }
        return;
    }
    let cols = if rtl { split(idx.clone(), |b| -b.x1(), |b| -b.x, boxes, tol) } else { split(idx.clone(), |b| b.x, |b| b.x1(), boxes, tol) };
    if cols.len() > 1 {
        for c in cols {
            rec(c, boxes, rtl, tol, out, depth + 1);
        }
        return;
    }
    // overlapping boxes (inset panels): top first, then reading side
    let mut idx = idx;
    idx.sort_by(|&a, &b| {
        let (p, q) = (&boxes[a], &boxes[b]);
        let x = if rtl { q.x1().total_cmp(&p.x1()) } else { p.x.total_cmp(&q.x) };
        p.y.total_cmp(&q.y).then(x)
    });
    out.extend(idx);
}

/// Order of the containers inside one panel.
pub fn balloon_order(idx: &[usize], boxes: &[Bounds], rtl: bool) -> Vec<usize> {
    let mut rest: Vec<usize> = idx.to_vec();
    let mut out = Vec::with_capacity(rest.len());
    while !rest.is_empty() {
        let top = *rest.iter().min_by(|&&a, &&b| boxes[a].y.total_cmp(&boxes[b].y)).unwrap();
        let mid = boxes[top].cy();
        let pick = *rest
            .iter()
            .filter(|&&i| boxes[i].y < mid)
            .min_by(|&&a, &&b| if rtl { boxes[b].x1().total_cmp(&boxes[a].x1()) } else { boxes[a].x.total_cmp(&boxes[b].x) })
            .unwrap_or(&top);
        out.push(pick);
        rest.retain(|&i| i != pick);
    }
    out
}

/// Panel of each container: the panel it overlaps most, if any.
pub fn assign_panels(panels: &[Bounds], items: &[Bounds]) -> Vec<Option<usize>> {
    items
        .iter()
        .map(|b| {
            panels
                .iter()
                .enumerate()
                .map(|(k, p)| (k, p.h_overlap(b).max(0.0) * p.v_overlap(b).max(0.0)))
                .filter(|(_, a)| *a > 0.0)
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(k, _)| k)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(x: f32, y: f32, w: f32, h: f32) -> Bounds {
        Bounds { x, y, w, h }
    }

    #[test]
    fn tiers_then_columns() {
        //  [ 0         ]
        //  [ 1 ][ 2 ][3]
        //  [ 4 ][  5   ]
        let p = vec![b(0.0, 0.0, 300.0, 100.0), b(0.0, 110.0, 90.0, 100.0), b(100.0, 110.0, 90.0, 100.0), b(200.0, 110.0, 100.0, 100.0), b(0.0, 220.0, 140.0, 100.0), b(150.0, 220.0, 150.0, 100.0)];
        assert_eq!(panel_order(&p, false, 3.0), vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(panel_order(&p, true, 3.0), vec![0, 3, 2, 1, 5, 4]);
    }

    #[test]
    fn tall_panel_beside_stacked_ones() {
        //  [   ][ 1 ]
        //  [ 0 ][ 2 ]
        let p = vec![b(0.0, 0.0, 100.0, 300.0), b(110.0, 0.0, 100.0, 140.0), b(110.0, 150.0, 100.0, 150.0)];
        assert_eq!(panel_order(&p, false, 3.0), vec![0, 1, 2]);
        assert_eq!(panel_order(&p, true, 3.0), vec![1, 2, 0]);
    }

    #[test]
    fn slightly_overlapping_tiers_still_cut() {
        let p = vec![b(0.0, 0.0, 100.0, 102.0), b(0.0, 100.0, 45.0, 100.0), b(55.0, 100.0, 45.0, 100.0)];
        assert_eq!(panel_order(&p, false, 4.0), vec![0, 1, 2]);
    }

    #[test]
    fn balloons_top_left_first() {
        // 0: right, slightly higher; 1: left, a bit lower; 2: bottom
        let bs = vec![b(200.0, 10.0, 80.0, 40.0), b(10.0, 20.0, 80.0, 40.0), b(100.0, 200.0, 80.0, 40.0)];
        assert_eq!(balloon_order(&[0, 1, 2], &bs, false), vec![1, 0, 2]);
        assert_eq!(balloon_order(&[0, 1, 2], &bs, true), vec![0, 1, 2]);
        // a balloon much lower on the left comes after the top-right one
        let bs = vec![b(200.0, 10.0, 80.0, 40.0), b(10.0, 80.0, 80.0, 40.0)];
        assert_eq!(balloon_order(&[0, 1], &bs, false), vec![0, 1]);
    }

    #[test]
    fn containers_go_to_the_most_overlapped_panel() {
        let panels = vec![b(0.0, 0.0, 100.0, 100.0), b(110.0, 0.0, 100.0, 100.0)];
        let items = vec![b(85.0, 10.0, 45.0, 20.0), b(400.0, 0.0, 10.0, 10.0)];
        assert_eq!(assign_panels(&panels, &items), vec![Some(1), None]);
    }
}
