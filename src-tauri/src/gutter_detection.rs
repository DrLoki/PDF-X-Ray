use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const MAX_RECURSION_DEPTH: usize = 24;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextElement {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub font_size: f32,
    pub font_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Formatting {
    pub fontFamily: String,
    pub fontSize: f32,
    pub avgFontSize: f32,
    pub fontWeight: String,
    pub fontStyle: String,
    pub color: String,
    pub alignment: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String, // "root", "container", "leaf"
    pub bounds: Bounds,
    pub depth: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cutDirection: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isBorderedBox: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formatting: Option<Formatting>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<LayoutNode>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gap {
    pub start: f32,
    pub end: f32,
    pub size: f32,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Projections {
    pub xGaps: Vec<Gap>,
    pub yGaps: Vec<Gap>,
    pub pageBounds: Bounds,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XYCutResult {
    pub root: LayoutNode,
    pub projections: Projections,
}

// Helper: Calculate statistical mode of float values
fn calculate_mode(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 10.0;
    }
    let mut counts = HashMap::new();
    for &v in values {
        let rounded = (v * 10.0).round() / 10.0;
        *counts.entry(rounded.to_bits()).or_insert(0) += 1;
    }
    let bits = counts
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .map(|(bits, _)| bits)
        .unwrap_or_else(|| 10.0f32.to_bits());
    f32::from_bits(bits)
}

// Strategy 1: Delta-X statistics & valley detection
pub fn detect_dynamic_column_gap(elements: &[TextElement], dominant_font: f32) -> f32 {
    if elements.len() < 2 {
        return dominant_font * 1.5;
    }

    // Group elements by baseline (tolerance of 4px)
    let mut sorted_by_y = elements.to_vec();
    sorted_by_y.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));

    let mut lines: Vec<Vec<TextElement>> = Vec::new();
    for el in sorted_by_y {
        let mut placed = false;
        for line in &mut lines {
            let avg_y: f32 = line.iter().map(|e| e.y).sum::<f32>() / line.len() as f32;
            if (el.y - avg_y).abs() < 4.0 {
                line.push(el.clone());
                placed = true;
                break;
            }
        }
        if !placed {
            lines.push(vec![el]);
        }
    }

    // Collect horizontal spacing (delta x) between neighboring elements in each line
    let mut deltas = Vec::new();
    for mut line in lines {
        line.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
        for i in 0..line.len() - 1 {
            let current = &line[i];
            let next = &line[i + 1];
            let delta = next.x - (current.x + current.width);
            if delta > 0.1 && delta < 150.0 {
                deltas.push(delta);
            }
        }
    }

    if deltas.is_empty() {
        return dominant_font * 1.5;
    }

    // Build histogram bins with 1pt resolution, up to 100pt
    let mut histogram = vec![0usize; 100];
    for d in deltas {
        let bin = d.round() as usize;
        if bin < histogram.len() {
            histogram[bin] += 1;
        }
    }

    // Peak B (Word Spacing) - highest count between 2pt and 10pt
    let mut peak_b_idx = 4;
    let mut max_b_freq = 0;
    for i in 2..=10 {
        if histogram[i] > max_b_freq {
            max_b_freq = histogram[i];
            peak_b_idx = i;
        }
    }

    // Peak C (Gutter) - highest count between (Peak B + 4) and 80pt
    let start_c = peak_b_idx + 4;
    let mut peak_c_idx = 0;
    let mut max_c_freq = 0;
    if start_c < 80 {
        for i in start_c..80 {
            if histogram[i] > max_c_freq {
                max_c_freq = histogram[i];
                peak_c_idx = i;
            }
        }
    }

    // If Peak C is found with a minimal size/frequency, look for the valley
    if peak_c_idx > peak_b_idx && max_c_freq >= 2 {
        let mut min_val = usize::MAX;
        let mut valley_idx = peak_b_idx + 2;
        for i in peak_b_idx..=peak_c_idx {
            if histogram[i] < min_val {
                min_val = histogram[i];
                valley_idx = i;
            }
        }
        return valley_idx as f32;
    }

    // Fallback heuristic: Font Dominante * 1.5
    dominant_font * 1.5
}

// Strategy 2: Project boxes onto an axis and find gaps
pub fn find_projection_gaps(
    elements: &[TextElement],
    axis: char,
    min_val: f32,
    max_val: f32,
) -> Vec<Gap> {
    if elements.is_empty() {
        return Vec::new();
    }

    // Create intervals
    let mut intervals: Vec<(f32, f32)> = elements
        .iter()
        .map(|el| {
            if axis == 'X' {
                (el.x, el.x + el.width)
            } else {
                (el.y, el.y + el.height)
            }
        })
        .collect();

    // Merge overlapping intervals
    intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged: Vec<(f32, f32)> = Vec::new();
    for interval in intervals {
        if merged.is_empty() {
            merged.push(interval);
        } else {
            let last_idx = merged.len() - 1;
            if interval.0 <= merged[last_idx].1 {
                merged[last_idx].1 = merged[last_idx].1.max(interval.1);
            } else {
                merged.push(interval);
            }
        }
    }

    // Find gaps between merged occupied intervals
    let mut gaps = Vec::new();

    // Gap before first interval
    if !merged.is_empty() && merged[0].0 > min_val {
        let size = merged[0].0 - min_val;
        if size > 0.1 {
            gaps.push(Gap {
                start: min_val,
                end: merged[0].0,
                size,
            });
        }
    }

    // Gaps between intervals
    for i in 0..merged.len() - 1 {
        let gap_start = merged[i].1;
        let gap_end = merged[i + 1].0;
        if gap_end > gap_start {
            gaps.push(Gap {
                start: gap_start,
                end: gap_end,
                size: gap_end - gap_start,
            });
        }
    }

    // Gap after last interval
    if !merged.is_empty() && merged[merged.len() - 1].1 < max_val {
        let size = max_val - merged[merged.len() - 1].1;
        if size > 0.1 {
            gaps.push(Gap {
                start: merged[merged.len() - 1].1,
                end: max_val,
                size,
            });
        }
    }

    gaps
}

// Helper to determine dominant styling and alignment for leaf nodes
fn finalize_leaf_node(
    id: String,
    elements: &[TextElement],
    bounds: Bounds,
    depth: usize,
) -> LayoutNode {
    if elements.is_empty() {
        return LayoutNode {
            id,
            node_type: "leaf".to_string(),
            bounds,
            depth,
            cutDirection: None,
            isBorderedBox: None,
            text: Some(String::new()),
            formatting: Some(Formatting {
                fontFamily: "default".to_string(),
                fontSize: 10.0,
                avgFontSize: 10.0,
                fontWeight: "normal".to_string(),
                fontStyle: "normal".to_string(),
                color: "#ffffff".to_string(),
                alignment: "left".to_string(),
            }),
            children: None,
        };
    }

    // Sort items for baseline reading flow
    let mut sorted_items = elements.to_vec();
    sorted_items.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));

    let mut rows: Vec<Vec<TextElement>> = Vec::new();
    for el in sorted_items {
        let mut placed = false;
        for row in &mut rows {
            let avg_y: f32 = row.iter().map(|e| e.y + e.height / 2.0).sum::<f32>() / row.len() as f32;
            let el_center = el.y + el.height / 2.0;
            if (el_center - avg_y).abs() < 5.0 {
                row.push(el.clone());
                placed = true;
                break;
            }
        }
        if !placed {
            rows.push(vec![el]);
        }
    }

    // Sort within each row by X coord
    for row in &mut rows {
        row.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    }

    // Sort rows by Y
    rows.sort_by(|a, b| {
        let avg_y_a = a.iter().map(|e| e.y).sum::<f32>() / a.len() as f32;
        let avg_y_b = b.iter().map(|e| e.y).sum::<f32>() / b.len() as f32;
        avg_y_a.partial_cmp(&avg_y_b).unwrap_or(std::cmp::Ordering::Equal)
    });

    let flattened: Vec<TextElement> = rows.clone().into_iter().flatten().collect();

    // Concatenate text segments
    let mut text_lines = Vec::new();
    for row in &rows {
        let row_text = row
            .iter()
            .map(|e| e.text.as_str())
            .collect::<Vec<&str>>()
            .join(" ");
        text_lines.push(row_text);
    }
    let text = text_lines.join("\n");

    // Extract dominant font size & family
    let mut font_sizes = HashMap::new();
    let mut font_families = HashMap::new();
    let mut total_font_size = 0.0;

    for item in &flattened {
        let size = item.font_size.round() as i32;
        *font_sizes.entry(size).or_insert(0) += 1;
        total_font_size += item.font_size;

        let family = item
            .font_name
            .clone()
            .unwrap_or_else(|| "default".to_string());
        *font_families.entry(family).or_insert(0) += 1;
    }

    let dominant_size = font_sizes
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .map(|(size, _)| size as f32)
        .unwrap_or(10.0);

    let dominant_family = font_families
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .map(|(fam, _)| fam)
        .unwrap_or_else(|| "default".to_string());

    let avg_size = (total_font_size / flattened.len() as f32 * 10.0).round() / 10.0;

    // Stylings from font name
    let family_lower = dominant_family.to_lowercase();
    let font_weight = if family_lower.contains("bold")
        || family_lower.contains("goth")
        || family_lower.contains("black")
    {
        "bold"
    } else {
        "normal"
    };

    let font_style = if family_lower.contains("italic") || family_lower.contains("oblique") {
        "italic"
    } else {
        "normal"
    };

    // Calculate alignment
    let mut alignment = "left".to_string();
    if !rows.is_empty() {
        let mut left_diff_sum = 0.0;
        let mut right_diff_sum = 0.0;
        let mut center_diff_sum = 0.0;
        let block_center_x = bounds.x + bounds.w / 2.0;

        for row in &rows {
            if row.is_empty() {
                continue;
            }
            let row_left = row[0].x;
            let row_right = row[row.len() - 1].x + row[row.len() - 1].width;
            let row_center_x = (row_left + row_right) / 2.0;

            left_diff_sum += (row_left - bounds.x).abs();
            right_diff_sum += (bounds.x + bounds.w - row_right).abs();
            center_diff_sum += (row_center_x - block_center_x).abs();
        }

        let avg_left_diff = left_diff_sum / rows.len() as f32;
        let avg_right_diff = right_diff_sum / rows.len() as f32;
        let avg_center_diff = center_diff_sum / rows.len() as f32;

        if avg_center_diff < 6.0 && avg_left_diff > 8.0 && avg_right_diff > 8.0 {
            alignment = "center".to_string();
        } else if avg_right_diff < 5.0 && avg_left_diff > 8.0 {
            alignment = "right".to_string();
        } else if avg_left_diff < 6.0 && avg_right_diff < 6.0 && rows.len() > 1 {
            alignment = "justify".to_string();
        }
    }

    LayoutNode {
        id,
        node_type: "leaf".to_string(),
        bounds,
        depth,
        cutDirection: None,
        isBorderedBox: None,
        text: Some(text),
        formatting: Some(Formatting {
            fontFamily: dominant_family,
            fontSize: dominant_size,
            avgFontSize: avg_size,
            fontWeight: font_weight.to_string(),
            fontStyle: font_style.to_string(),
            color: "#ffffff".to_string(),
            alignment,
        }),
        children: None,
    }
}

// Recursive subdivisions
fn subdivide_node(
    elements: &[TextElement],
    bounds: Bounds,
    depth: usize,
    id_counter: &mut usize,
    strategy: &str,
) -> LayoutNode {
    let font_sizes: Vec<f32> = elements.iter().map(|e| e.font_size).collect();
    let dominant_font = calculate_mode(&font_sizes);

    // Prevent excessive recursion depth which may cause stack overflow
    if depth > MAX_RECURSION_DEPTH {
        let my_id = if depth == 0 {
            "block-root".to_string()
        } else {
            *id_counter += 1;
            format!("block-{}", *id_counter)
        };
        return finalize_leaf_node(my_id, elements, bounds, depth);
    }

    // Min dimensions validation
    let min_width = 20.0;
    let min_height = 10.0;

    let my_id = if depth == 0 {
        "block-root".to_string()
    } else {
        *id_counter += 1;
        format!("block-{}", id_counter)
    };

    if elements.len() <= 1 || bounds.w < min_width || bounds.h < min_height {
        return finalize_leaf_node(my_id, elements, bounds, depth);
    }

    // Select thresholds based on selected strategy
    let gutter_min = dominant_font * 1.5;
    let (dynamic_threshold_x, dynamic_threshold_y) = match strategy {
        "delta-x" => (
            detect_dynamic_column_gap(elements, dominant_font),
            (dominant_font * 1.0).max(4.0),
        ),
        "zero-run" => (
            10.0,
            5.0,
        ),
        "dominant-font" => (
            dominant_font * 1.5,
            dominant_font * 1.0,
        ),
        _ => ( // "combined"
            detect_dynamic_column_gap(elements, dominant_font).max(gutter_min),
            (dominant_font * 1.0).max(4.0),
        ),
    };

    // Find gaps on X and Y axes
    let min_x = bounds.x;
    let max_x = bounds.x + bounds.w;
    let min_y = bounds.y;
    let max_y = bounds.y + bounds.h;

    let x_gaps: Vec<Gap> = find_projection_gaps(elements, 'X', min_x, max_x)
        .into_iter()
        .filter(|g| g.size >= dynamic_threshold_x)
        .collect();

    let y_gaps: Vec<Gap> = find_projection_gaps(elements, 'Y', min_y, max_y)
        .into_iter()
        .filter(|g| g.size >= dynamic_threshold_y)
        .collect();

    // If no valid gaps exceed the dynamic thresholds, this is a leaf node
    if x_gaps.is_empty() && y_gaps.is_empty() {
        return finalize_leaf_node(my_id, elements, bounds, depth);
    }

    // Centered Title Heuristic (Title Centrati Passanti) & Footnotes check:
    // We prioritize Y-Cut (horizontal first) if Y-gaps exist, to isolate title blocks
    // and horizontal sections before vertical splitting of columns.
    let cut_direction = if !y_gaps.is_empty() {
        'Y'
    } else {
        'X'
    };

    let mut children = Vec::new();

    if cut_direction == 'X' {
        let mut selected_gaps = x_gaps;
        selected_gaps.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());

        let mut current_x = min_x;
        for gap in selected_gaps {
            // Validation: Column cuts must not be too close to outer margins
            // & should be in division zones (around 1/2, or 1/3 and 2/3)
            let mid_gap = (gap.start + gap.end) / 2.0;
            let rel_pos = (mid_gap - min_x) / bounds.w;
            
            // If it's a very narrow margin cut (e.g. within outer 5% of boundaries), ignore it to prevent empty margin splits
            if rel_pos < 0.05 || rel_pos > 0.95 {
                current_x = gap.end;
                continue;
            }

            if gap.start > current_x {
                let sub_width = gap.start - current_x;

                // Skip trivial subdivisions that don't reduce the element set or are too small
                if sub_width < 1.0 || sub_width < min_width {
                    current_x = gap.end;
                    continue;
                }

                let sub_items: Vec<TextElement> = elements
                    .iter()
                    .filter(|item| item.x >= current_x && (item.x + item.width) <= gap.start)
                    .cloned()
                    .collect();

                // If the subdivision doesn't reduce the element count, skip to avoid deep identical nesting
                if sub_items.is_empty() || sub_items.len() == elements.len() {
                    current_x = gap.end;
                    continue;
                }

                let child_bounds = Bounds {
                    x: current_x,
                    y: min_y,
                    w: sub_width,
                    h: bounds.h,
                };
                children.push(subdivide_node(&sub_items, child_bounds, depth + 1, id_counter, strategy));
            }
            current_x = gap.end;
        }

        if max_x > current_x {
            let sub_width = max_x - current_x;

            if sub_width >= 1.0 && sub_width >= min_width {
                let sub_items: Vec<TextElement> = elements
                    .iter()
                    .filter(|item| item.x >= current_x && (item.x + item.width) <= max_x)
                    .cloned()
                    .collect();

                if !sub_items.is_empty() && sub_items.len() != elements.len() {
                    let child_bounds = Bounds {
                        x: current_x,
                        y: min_y,
                        w: sub_width,
                        h: bounds.h,
                    };
                    children.push(subdivide_node(&sub_items, child_bounds, depth + 1, id_counter, strategy));
                }
            }
        }
    } else {
        // cut_direction == 'Y'
        let mut selected_gaps = y_gaps;
        selected_gaps.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());

        let mut current_y = min_y;
        for gap in selected_gaps {
            if gap.start > current_y {
                let sub_height = gap.start - current_y;

                // Skip trivial subdivisions
                if sub_height < 1.0 || sub_height < min_height {
                    current_y = gap.end;
                    continue;
                }

                let sub_items: Vec<TextElement> = elements
                    .iter()
                    .filter(|item| item.y >= current_y && (item.y + item.height) <= gap.start)
                    .cloned()
                    .collect();

                if sub_items.is_empty() || sub_items.len() == elements.len() {
                    current_y = gap.end;
                    continue;
                }

                let child_bounds = Bounds {
                    x: min_x,
                    y: current_y,
                    w: bounds.w,
                    h: sub_height,
                };
                children.push(subdivide_node(&sub_items, child_bounds, depth + 1, id_counter, strategy));
            }
            current_y = gap.end;
        }

        if max_y > current_y {
            let sub_height = max_y - current_y;
            if sub_height >= 1.0 && sub_height >= min_height {
                let sub_items: Vec<TextElement> = elements
                    .iter()
                    .filter(|item| item.y >= current_y && (item.y + item.height) <= max_y)
                    .cloned()
                    .collect();

                if !sub_items.is_empty() && sub_items.len() != elements.len() {
                    let child_bounds = Bounds {
                        x: min_x,
                        y: current_y,
                        w: bounds.w,
                        h: sub_height,
                    };
                    children.push(subdivide_node(&sub_items, child_bounds, depth + 1, id_counter, strategy));
                }
            }
        }
    }

    if children.is_empty() {
        return finalize_leaf_node(my_id, elements, bounds, depth);
    }

    LayoutNode {
        id: my_id,
        node_type: "container".to_string(),
        bounds,
        depth,
        cutDirection: Some(cut_direction.to_string()),
        isBorderedBox: None,
        text: None,
        formatting: None,
        children: Some(children),
    }
}

// Entrypoint
pub fn perform_auto_xycut(
    items: &[TextElement],
    page_bounds: Bounds,
    bordered_boxes: &[Bounds],
    strategy: &str,
) -> XYCutResult {
    let mut id_counter = 0;
    let mut root_children = Vec::new();
    let mut remaining_items = items.to_vec();

    // 1. Process Footnotes preprocessing
    // Check if there are footnote elements: y > 85% of height and size < page mode font size
    let font_sizes: Vec<f32> = items.iter().map(|e| e.font_size).collect();
    let doc_mode_font = calculate_mode(&font_sizes);
    let footnote_limit_y = page_bounds.y + page_bounds.h * 0.85;

    let footnote_items: Vec<TextElement> = remaining_items
        .iter()
        .filter(|e| e.y >= footnote_limit_y && e.font_size < doc_mode_font)
        .cloned()
        .collect();

    let mut footnote_block = None;
    if !footnote_items.is_empty() {
        // Extract minimum Y starting point of the footnote items
        let footnote_min_y = footnote_items
            .iter()
            .map(|e| e.y)
            .fold(page_bounds.y + page_bounds.h, f32::min);

        // Filter these out from the main body items
        remaining_items.retain(|e| {
            !footnote_items
                .iter()
                .any(|f| (f.x - e.x).abs() < 0.1 && (f.y - e.y).abs() < 0.1)
        });

        id_counter += 1;
        let fn_bounds = Bounds {
            x: page_bounds.x,
            y: footnote_min_y,
            w: page_bounds.w,
            h: page_bounds.y + page_bounds.h - footnote_min_y,
        };
        
        footnote_block = Some(subdivide_node(
            &footnote_items,
            fn_bounds,
            1,
            &mut id_counter,
            strategy,
        ));
    }

    // 2. Process Bordered Boxes (Callout boxes / graphical blocks)
    let mut sorted_boxes = bordered_boxes.to_vec();
    sorted_boxes.sort_by(|a, b| (b.w * b.h).partial_cmp(&(a.w * a.h)).unwrap());

    for box_bounds in sorted_boxes {
        let inside_items: Vec<TextElement> = remaining_items
            .iter()
            .filter(|item| {
                let cx = item.x + item.width / 2.0;
                let cy = item.y + item.height / 2.0;
                cx >= box_bounds.x
                    && cx <= box_bounds.x + box_bounds.w
                    && cy >= box_bounds.y
                    && cy <= box_bounds.y + box_bounds.h
            })
            .cloned()
            .collect();

        if !inside_items.is_empty() {
            // Remove these items from remaining_items
            remaining_items.retain(|item| {
                !inside_items
                    .iter()
                    .any(|ins| (ins.x - item.x).abs() < 0.1 && (ins.y - item.y).abs() < 0.1)
            });

            id_counter += 1;
            let mut box_block = LayoutNode {
                id: format!("bordered-box-{}", id_counter),
                node_type: "container".to_string(),
                bounds: box_bounds,
                depth: 1,
                cutDirection: None,
                isBorderedBox: Some(true),
                text: None,
                formatting: None,
                children: None,
            };

            let subdivided = subdivide_node(&inside_items, box_block.bounds.clone(), 1, &mut id_counter, strategy);
            box_block.children = subdivided.children;
            box_block.cutDirection = subdivided.cutDirection;
            root_children.push(box_block);
        }
    }

    // 3. Process remaining items
    if !remaining_items.is_empty() {
        // Main body bounds should be adjusted to end above the footnote
        let main_h = if let Some(ref fn_blk) = footnote_block {
            fn_blk.bounds.y - page_bounds.y
        } else {
            page_bounds.h
        };

        let temp_root_bounds = Bounds {
            x: page_bounds.x,
            y: page_bounds.y,
            w: page_bounds.w,
            h: main_h,
        };

        let body_tree = subdivide_node(&remaining_items, temp_root_bounds, 0, &mut id_counter, strategy);
        if let Some(children) = body_tree.children {
            root_children.extend(children);
        } else if body_tree.node_type == "leaf" {
            root_children.push(body_tree);
        }
    }

    // Add footnote block at the end if it exists
    if let Some(fn_block) = footnote_block {
        root_children.push(fn_block);
    }

    let root_node = LayoutNode {
        id: "block-root".to_string(),
        node_type: "root".to_string(),
        bounds: page_bounds.clone(),
        depth: 0,
        cutDirection: None,
        isBorderedBox: None,
        text: None,
        formatting: None,
        children: Some(root_children),
    };

    let x_gaps = find_projection_gaps(items, 'X', page_bounds.x, page_bounds.x + page_bounds.w);
    let y_gaps = find_projection_gaps(items, 'Y', page_bounds.y, page_bounds.y + page_bounds.h);

    XYCutResult {
        root: root_node,
        projections: Projections {
            xGaps: x_gaps,
            yGaps: y_gaps,
            pageBounds: page_bounds,
        },
    }
}
