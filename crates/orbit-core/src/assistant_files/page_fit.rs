//! Cutting an answer to the page of the bridge that carries it, by the bytes
//! its JSON costs, and saying where the next page starts.
use super::{Entry, MAX_PAGE_BYTES, PdfPage, SearchHit, SearchLimit, SearchStop};
use serde::Serialize;

/// The bytes one value costs inside a JSON array, its comma included.
fn element_cost(value: &impl Serialize) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len().saturating_add(1))
}

/// The entries that fit `budget`, in order, and where the listing goes on.
/// At least one entry while any remain, so a walk over pages ends.
pub fn fit_entries(
    entries: Vec<Entry>,
    after_index: u64,
    next_after_index: Option<u64>,
    budget: usize,
) -> (Vec<Entry>, Option<u64>) {
    let mut used = 0_usize;
    let mut kept = Vec::new();
    let total = entries.len();
    for entry in entries {
        let cost = element_cost(&entry);
        if !kept.is_empty() && used.saturating_add(cost) > budget {
            break;
        }
        used = used.saturating_add(cost);
        kept.push(entry);
    }
    if kept.len() < total {
        let next = after_index.saturating_add(kept.len() as u64);
        return (kept, Some(next));
    }
    (kept, next_after_index)
}

/// The matches that fit `budget`; a search cut here says so as its stop.
pub fn fit_hits(
    results: Vec<SearchHit>,
    stopped: Option<SearchStop>,
    budget: usize,
) -> (Vec<SearchHit>, Option<SearchStop>) {
    let mut used = 0_usize;
    let mut kept = Vec::new();
    let total = results.len();
    for hit in results {
        let cost = element_cost(&hit);
        if used.saturating_add(cost) > budget {
            break;
        }
        used = used.saturating_add(cost);
        kept.push(hit);
    }
    if kept.len() < total {
        return (
            kept,
            Some(SearchStop {
                cause: SearchLimit::PageBytes,
                limit: budget as u64,
            }),
        );
    }
    (kept, stopped)
}

/// The text that fits `budget` and where the file goes on.
pub fn fit_text(
    text: String,
    after_index: u64,
    next_after_index: Option<u64>,
    budget: usize,
) -> (String, Option<u64>) {
    let keep = fitting_prefix(&text, budget);
    if keep < text.len() {
        let next = after_index.saturating_add(keep as u64);
        let mut text = text;
        text.truncate(keep);
        return (text, Some(next));
    }
    (text, next_after_index)
}

/// What one page entry costs besides its text: `{"page":N,"text":},`.
pub(super) const PAGE_ENVELOPE_BYTES: usize = 24;

/// The pages that fit `budget`, the last one cut at a character when it
/// does not fit whole, and where the document goes on: the page (from 0)
/// and the byte of its text.
pub fn fit_pages(
    pages: Vec<PdfPage>,
    after_page: u32,
    after_index: u64,
    next: Option<(u32, u64)>,
    budget: usize,
) -> (Vec<PdfPage>, Option<(u32, u64)>) {
    let mut used = 0_usize;
    let mut kept = Vec::new();
    for page in pages {
        let index = page.page.saturating_sub(1);
        let starts_at = if index == after_page { after_index } else { 0 };
        let cost = encoded_len(&page.text).saturating_add(PAGE_ENVELOPE_BYTES);
        if used.saturating_add(cost) <= budget {
            used = used.saturating_add(cost);
            kept.push(page);
            continue;
        }
        let room = budget
            .saturating_sub(used)
            .saturating_sub(PAGE_ENVELOPE_BYTES);
        let keep = fitting_prefix(&page.text, room);
        let resume = (index, starts_at.saturating_add(keep as u64));
        if keep > 0 {
            let mut page = page;
            page.text.truncate(keep);
            kept.push(page);
        }
        return (kept, Some(resume));
    }
    (kept, next)
}

/// What a page of an answer can spend on its results once the path it echoes
/// and its envelope are paid for.
pub fn page_budget(path: &str) -> usize {
    MAX_PAGE_BYTES
        .saturating_sub(encoded_len(path))
        .saturating_sub(1024)
}

/// The JSON bytes one character costs inside a string, as `serde_json` writes it.
fn escaped(character: char) -> usize {
    match character {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{08}' | '\u{0C}' => 2,
        control if u32::from(control) < 0x20 => 6,
        other => other.len_utf8(),
    }
}

/// The JSON bytes a string costs, quotes included.
pub fn encoded_len(text: &str) -> usize {
    text.chars().map(escaped).sum::<usize>().saturating_add(2)
}

/// How many bytes of `text` fit `budget` once encoded as a JSON string, cut
/// at a character boundary. Every page budget holds thousands of the
/// costliest character (six bytes), so a walk over pages always moves.
pub fn fitting_prefix(text: &str, budget: usize) -> usize {
    let mut used = 2_usize;
    for (index, character) in text.char_indices() {
        let cost = escaped(character);
        if used.saturating_add(cost) > budget {
            return index;
        }
        used = used.saturating_add(cost);
    }
    text.len()
}
