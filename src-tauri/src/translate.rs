//! Machine translation through the free Google Translate endpoint — the
//! "free" mode of GianoReader, ported (same endpoint, limits and batching).
//!
//! The endpoint only accepts GET requests with the text in the URL and
//! rejects URLs longer than about 16 KB; every UTF-8 byte of a non-ASCII
//! character becomes three URL characters, so the limit is counted in UTF-8
//! bytes ([`BYTE_LIMIT`]). Paragraphs are sent in batches joined by blank
//! lines and split back on them; when the engine does not keep the blank
//! lines (short lines of dialogue often get merged), the paragraphs of that
//! batch are translated one by one, so nothing is lost or misaligned. A
//! paragraph over the limit is sent in several pieces, cut after whitespace
//! or punctuation.
//!
//! The requests run in the Rust backend: from the WebView they would be
//! subject to CORS (a rate-limit 429 from Google has no CORS header and
//! looks like a CORS failure).

use serde_json::Value;
use std::time::Duration;

/// Maximum UTF-8 bytes of text per request (at most 13.5 KB once encoded).
pub const BYTE_LIMIT: usize = 4500;

/// Google's code for an internal language code (`pt` is Portugal for us,
/// Brazil for Google). Same mapping as GianoReader.
pub fn google_lang_code(lang: &str) -> &str {
    match lang {
        "pt" => "pt-PT",
        "pt-BR" => "pt",
        other => other,
    }
}

/// Percent-encoding as JavaScript's `encodeURIComponent`.
fn encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Split `text` into pieces of at most `limit` bytes, cutting after the last
/// whitespace or punctuation that fits (between characters for a long CJK
/// run). Joining the pieces gives back `text`.
pub fn split_by_byte_limit(text: &str, limit: usize) -> Vec<&str> {
    if text.len() <= limit {
        return vec![text];
    }
    let is_break = |c: char| c.is_whitespace() || matches!(c, '.' | '!' | '?' | ';' | ':' | ',' | '。' | '！' | '？' | '；' | '：' | '，' | '、' | '…');
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut break_at = None;
    for (offset, c) in text.char_indices() {
        let end = offset + c.len_utf8();
        while end - start > limit && offset > start {
            let cut = match break_at {
                Some(b) if b > start => b,
                _ => offset,
            };
            pieces.push(&text[start..cut]);
            start = cut;
            break_at = None;
        }
        if is_break(c) {
            break_at = Some(end);
        }
    }
    if start < text.len() {
        pieces.push(&text[start..]);
    }
    pieces
}

/// Batches of consecutive paragraphs whose blank-line join fits the limit:
/// `(first, end)` index ranges.
pub fn batches(texts: &[String]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let (mut start, mut len) = (0usize, 0usize);
    for (i, t) in texts.iter().enumerate() {
        let add = if i > start { 2 + t.len() } else { t.len() };
        if i > start && len + add > BYTE_LIMIT {
            out.push((start, i));
            start = i;
            len = t.len();
        } else {
            len += add;
        }
    }
    if start < texts.len() {
        out.push((start, texts.len()));
    }
    out
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(60)))
        .http_status_as_error(false)
        .build()
        .into()
}

/// One GET request.
fn request_once(agent: &ureq::Agent, text: &str, source: &str, target: &str) -> Result<String, String> {
    let url = format!(
        "https://translate.googleapis.com/translate_a/single?client=gtx&sl={}&tl={}&dt=t&q={}",
        encode(source),
        encode(google_lang_code(target)),
        encode(text)
    );
    let mut resp = agent.get(&url).call().map_err(|e| format!("rete: {e}"))?;
    let status = resp.status().as_u16();
    if status == 429 {
        return Err("troppe richieste a Google Translate (HTTP 429): riprova tra qualche minuto".into());
    }
    if !(200..300).contains(&status) {
        return Err(format!("traduzione: HTTP {status}"));
    }
    let body = resp.body_mut().read_to_string().map_err(|e| format!("risposta non valida: {e}"))?;
    let data: Value = serde_json::from_str(&body).map_err(|e| format!("risposta non valida: {e}"))?;
    let segments = data.get(0).and_then(|v| v.as_array()).ok_or("risposta non valida: segmenti mancanti")?;
    Ok(segments.iter().filter_map(|s| s.get(0).and_then(|v| v.as_str())).collect())
}

/// Translate one text (split when over the limit).
fn translate_text(agent: &ureq::Agent, text: &str, source: &str, target: &str) -> Result<String, String> {
    let pieces = split_by_byte_limit(text, BYTE_LIMIT);
    if pieces.len() == 1 {
        return request_once(agent, text, source, target);
    }
    let mut out = Vec::with_capacity(pieces.len());
    for p in pieces {
        out.push(request_once(agent, p, source, target)?.trim().to_string());
    }
    Ok(out.join(" "))
}

/// Translate paragraphs (`source` may be `auto`). `progress(done, total)`
/// is called after each request batch, with paragraphs as the unit.
pub fn translate_free(texts: &[String], source: &str, target: &str, progress: &mut dyn FnMut(usize, usize)) -> Result<Vec<String>, String> {
    let agent = agent();
    let mut out = vec![String::new(); texts.len()];
    for (start, end) in batches(texts) {
        let joined = texts[start..end].join("\n\n");
        let translated = translate_text(&agent, &joined, source, target)?;
        let parts: Vec<&str> = translated.split("\n\n").map(str::trim).filter(|p| !p.is_empty()).collect();
        if parts.len() == end - start {
            for (k, p) in parts.into_iter().enumerate() {
                out[start + k] = p.to_string();
            }
        } else {
            // the blank lines were not kept: one paragraph per request
            for i in start..end {
                out[i] = translate_text(&agent, &texts[i], source, target)?.trim().to_string();
            }
        }
        progress(end, texts.len());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_respect_the_byte_limit() {
        let texts: Vec<String> = (0..10).map(|i| format!("{i}").repeat(1000)).collect();
        let b = batches(&texts);
        assert_eq!(b.first().unwrap().0, 0);
        assert_eq!(b.last().unwrap().1, 10);
        for &(s, e) in &b {
            let joined = texts[s..e].join("\n\n");
            assert!(joined.len() <= BYTE_LIMIT || e - s == 1);
        }
        assert_eq!(b.len(), 3); // 4 + 4 + 2 paragraphs of 1000 bytes
    }

    #[test]
    fn long_cjk_text_is_split_by_bytes() {
        let text = "考えているんです。".repeat(400); // 27 bytes × 400
        let pieces = split_by_byte_limit(&text, BYTE_LIMIT);
        assert!(pieces.len() > 1);
        assert!(pieces.iter().all(|p| p.len() <= BYTE_LIMIT && p.ends_with('。')));
        assert_eq!(pieces.concat(), text);
    }

    #[test]
    fn encoding_and_codes() {
        assert_eq!(encode("a b&c/é"), "a%20b%26c%2F%C3%A9");
        assert_eq!(google_lang_code("pt"), "pt-PT");
        assert_eq!(google_lang_code("it"), "it");
    }
}
