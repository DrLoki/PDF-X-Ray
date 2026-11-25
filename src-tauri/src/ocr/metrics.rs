//! Error rates of an OCR result against a reference transcription.
//!
//! Both sides are lists of lines. Reading order is not part of the score
//! (it belongs to the layout engine): every reference line is paired with
//! the recognised line closest to it (edit distance relative to its length,
//! best pairs first, one-to-one). Then
//!
//! - **CER** = (edits in the pairs + characters of unpaired reference lines
//!   + characters of unpaired recognised lines) / reference characters;
//! - **WER**, the same over words.
//!
//! Text is normalised first: ligatures, typographic quotes, dashes and
//! ellipsis are mapped to their plain forms and whitespace is collapsed, so
//! the score measures recognition, not typography.

use crate::text::normalize_fragment;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OcrScore {
    pub cer: f32,
    pub wer: f32,
    pub reference_lines: usize,
    pub reference_chars: usize,
    pub recognised_lines: usize,
    pub paired_lines: usize,
    /// Reference lines with no recognised counterpart (missed text).
    pub missed_lines: usize,
    /// Recognised lines with no reference counterpart (spurious text).
    pub extra_lines: usize,
}

pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in normalize_fragment(s).chars() {
        match c {
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{2032}' | '`' | '\u{00B4}' => out.push('\''),
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' | '\u{2033}' | '\u{00AB}' | '\u{00BB}' => out.push('"'),
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}' | '\u{2212}' => out.push('-'),
            '\u{2026}' => out.push_str("..."),
            '\u{00AD}' => {}
            c if c.is_whitespace() => out.push(' '),
            c => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn levenshtein<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, x) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + usize::from(x != y));
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

pub fn score(reference: &[String], recognised: &[String]) -> OcrScore {
    let refs: Vec<Vec<char>> = reference.iter().map(|s| normalize(s).chars().collect()).filter(|v: &Vec<char>| !v.is_empty()).collect();
    let hyps: Vec<Vec<char>> = recognised.iter().map(|s| normalize(s).chars().collect()).filter(|v: &Vec<char>| !v.is_empty()).collect();
    let words = |v: &Vec<char>| v.iter().collect::<String>().split(' ').map(str::to_string).collect::<Vec<_>>();
    let ref_words: Vec<Vec<String>> = refs.iter().map(words).collect();
    let hyp_words: Vec<Vec<String>> = hyps.iter().map(words).collect();

    // candidate pairs, best (relative distance) first
    let mut cands: Vec<(f32, usize, usize, usize)> = Vec::new();
    for (i, r) in refs.iter().enumerate() {
        for (j, h) in hyps.iter().enumerate() {
            // cheap bound: the length difference alone already rules the pair out
            if r.len().abs_diff(h.len()) >= r.len() {
                continue;
            }
            let d = levenshtein(r, h);
            if d < r.len() {
                cands.push((d as f32 / r.len() as f32, i, j, d));
            }
        }
    }
    cands.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut ref_used = vec![false; refs.len()];
    let mut hyp_used = vec![false; hyps.len()];
    let (mut char_err, mut word_err, mut paired) = (0usize, 0usize, 0usize);
    for (_, i, j, d) in cands {
        if ref_used[i] || hyp_used[j] {
            continue;
        }
        ref_used[i] = true;
        hyp_used[j] = true;
        paired += 1;
        char_err += d;
        word_err += levenshtein(&ref_words[i], &hyp_words[j]);
    }
    let mut missed = 0;
    for (i, r) in refs.iter().enumerate() {
        if !ref_used[i] {
            missed += 1;
            char_err += r.len();
            word_err += ref_words[i].len();
        }
    }
    let mut extra = 0;
    for (j, h) in hyps.iter().enumerate() {
        if !hyp_used[j] {
            extra += 1;
            char_err += h.len();
            word_err += hyp_words[j].len();
        }
    }
    let ref_chars: usize = refs.iter().map(|r| r.len()).sum();
    let ref_word_count: usize = ref_words.iter().map(|w| w.len()).sum();
    let rate = |e: usize, n: usize| if n == 0 { if e == 0 { 0.0 } else { 1.0 } } else { e as f32 / n as f32 };
    OcrScore {
        cer: rate(char_err, ref_chars),
        wer: rate(word_err, ref_word_count),
        reference_lines: refs.len(),
        reference_chars: ref_chars,
        recognised_lines: hyps.len(),
        paired_lines: paired,
        missed_lines: missed,
        extra_lines: extra,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn identical_text_in_another_order_scores_zero() {
        let s = score(&v(&["first line", "second line"]), &v(&["second line", "first  line"]));
        assert_eq!(s.cer, 0.0);
        assert_eq!(s.wer, 0.0);
        assert_eq!(s.paired_lines, 2);
    }

    #[test]
    fn typography_is_not_an_error() {
        let s = score(&v(&["\u{201C}living handout,\u{201D} it\u{2019}s \u{FB01}ne\u{2026}"]), &v(&["\"living handout,\" it's fine..."]));
        assert_eq!(s.cer, 0.0);
    }

    #[test]
    fn substitutions_missed_and_extra_lines_count() {
        // "tor" vs "for": 1 edit over 25 + missed "A" (1) + extra "xx" (2)
        let s = score(&v(&["the roots for extending p", "A"]), &v(&["the roots tor extending p", "xx"]));
        assert_eq!(s.paired_lines, 1);
        assert_eq!(s.missed_lines, 1);
        assert_eq!(s.extra_lines, 1);
        assert!((s.cer - 4.0 / 26.0).abs() < 1e-6, "{}", s.cer);
        assert!((s.wer - 3.0 / 6.0).abs() < 1e-6, "{}", s.wer);
    }
}
