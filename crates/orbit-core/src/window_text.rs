//! The window's text «Adjuntar ventana» attaches next to its capture: what
//! Accessibility shows in that window, read by the engine when it runs and
//! otherwise by the helper (`window_text`), kept within its ceiling. The
//! text is the user's content: it never goes to a log or to evidence.
use serde::{Deserialize, Serialize};

/// The most bytes of a window's text kept as its attachment.
///
/// Protects: the helper's answer, one stdout line of at most 1 MiB
/// (`NativeLimits.maximumEventLineBytes`). JSON writes a control character
/// as six bytes, so 128 KiB of the worst text is 768 KiB and still fits with
/// its envelope. Why 128 KiB: some forty pages of prose, more than one
/// window shows. It does not limit the capture or an attached file, which
/// have their own ceilings; a longer text is cut and says so.
pub const MAX_WINDOW_TEXT_BYTES: u32 = 128 * 1024;

/// The most Accessibility elements the helper visits in one window.
///
/// Protects: the helper's time, since each element is a call into the
/// window's app. Why 2 000: what the engine walks for one window
/// (cua-driver 0.28.2, `scripts/e2e-mac/interface.py`). Not a limit on the
/// text: a window with more elements is cut there and says so.
pub const MAX_WINDOW_TEXT_NODES: u32 = 2_000;

/// How long the helper walks one window, in milliseconds.
///
/// Protects: the person waiting on «Adjuntar ventana» while an app answers
/// Accessibility slowly. Why 3 s: a judgement, not measured; a window read
/// in full takes well under a second when its app answers. The helper stops
/// there with what it read, which says it was cut. It does not limit the
/// capture.
pub const WINDOW_TEXT_DEADLINE_MS: u64 = 3_000;

/// The bounds the daemon gives the helper's walk (`window_text`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowTextLimits {
    pub max_bytes: u32,
    pub max_nodes: u32,
    pub deadline_ms: u64,
}

impl WindowTextLimits {
    pub const ATTACHMENT: Self = Self {
        max_bytes: MAX_WINDOW_TEXT_BYTES,
        max_nodes: MAX_WINDOW_TEXT_NODES,
        deadline_ms: WINDOW_TEXT_DEADLINE_MS,
    };
}

/// Which bound stopped the helper's walk before the window's end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowTextStop {
    Bytes,
    Nodes,
    Deadline,
}

/// The helper's answer to `window_text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowTextReply {
    /// One line per element that shows text, in reading order.
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped: Option<WindowTextStop>,
}

/// The last line of a text that was cut: the agent knows it has the start.
pub const CUT_NOTE: &str = "\n\n[La ventana tiene más texto; Orbit adjuntó sólo el principio.]";

/// The text kept from what was read: `None` when nothing readable is left;
/// within `MAX_WINDOW_TEXT_BYTES`, cut on a character boundary, and with
/// `CUT_NOTE` last when the window had more than was read (`cut`) or more
/// than the ceiling.
pub fn kept(text: &str, cut: bool) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let ceiling = MAX_WINDOW_TEXT_BYTES as usize;
    if !cut && text.len() <= ceiling {
        return Some(text.to_owned());
    }
    let mut end = text.len().min(ceiling - CUT_NOTE.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Some(format!("{}{CUT_NOTE}", text[..end].trim_end()))
}

/// The engine's tree of one window (`get_window_state`'s `tree_markdown`,
/// two spaces of indent per level) without the app's menu bar and what
/// hangs from it: the menus are the app's, not the window's, and «Abrir
/// reciente» lists other documents.
pub fn without_menu_bar(tree: &str) -> String {
    let mut kept = String::with_capacity(tree.len());
    // The indent of the menu bar being left out.
    let mut inside: Option<usize> = None;
    for line in tree.split_inclusive('\n') {
        let indent = line.len() - line.trim_start_matches(' ').len();
        let rest = &line[indent..];
        let node = rest.starts_with("- ");
        if let Some(bar) = inside {
            // A line that is not a node continues the text of the one above.
            if !node || indent > bar {
                continue;
            }
            inside = None;
        }
        let role = rest
            .strip_prefix("- ")
            .map(|after| match after.strip_prefix('[') {
                Some(indexed) => indexed.split_once("] ").map_or(after, |(_, role)| role),
                None => after,
            })
            .unwrap_or_default();
        let role = role.split_whitespace().next().unwrap_or_default();
        if node && role == "AXMenuBar" {
            inside = Some(indent);
            continue;
        }
        kept.push_str(line);
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_without_text_keeps_nothing() {
        assert_eq!(kept("", false), None);
        assert_eq!(kept(" \n\t\n", true), None);
    }

    #[test]
    fn a_short_text_is_kept_whole() {
        assert_eq!(
            kept("\nLista de la compra\nPan\n", false),
            Some("Lista de la compra\nPan".into())
        );
    }

    #[test]
    fn a_text_over_the_ceiling_is_cut_on_a_character_and_says_so() {
        let long = "ñ".repeat(MAX_WINDOW_TEXT_BYTES as usize);
        let text = kept(&long, false).unwrap_or_default();
        assert!(
            text.len() <= MAX_WINDOW_TEXT_BYTES as usize,
            "{}",
            text.len()
        );
        assert!(text.ends_with(CUT_NOTE));
        assert!(text.trim_end_matches(CUT_NOTE).chars().all(|c| c == 'ñ'));
    }

    #[test]
    fn a_walk_the_helper_stopped_says_the_window_has_more() {
        assert_eq!(
            kept("Primera línea", true),
            Some(format!("Primera línea{CUT_NOTE}"))
        );
    }

    #[test]
    fn the_engine_tree_leaves_the_menu_bar_out() {
        let tree = "- AXWindow \"Notas\"\n  - AXMenuBar\n    - [3] AXMenuBarItem \"Archivo\"\n      - AXMenuItem \"Abrir reciente\"\n  - AXStaticText = \"Lista de la compra\"\n  - [4] AXMenuBar \"otra\"\n    - AXMenuItem \"x\"\n  - AXStaticText = \"una línea\nque sigue\"\n";
        assert_eq!(
            without_menu_bar(tree),
            "- AXWindow \"Notas\"\n  - AXStaticText = \"Lista de la compra\"\n  - AXStaticText = \"una línea\nque sigue\"\n"
        );
    }

    #[test]
    fn the_helper_s_answer_reads_with_and_without_its_stop() -> Result<(), serde_json::Error> {
        let whole: WindowTextReply = serde_json::from_value(serde_json::json!({"text":"Hola"}))?;
        assert_eq!(whole.stopped, None);
        let cut: WindowTextReply =
            serde_json::from_value(serde_json::json!({"text":"Hola","stopped":"deadline"}))?;
        assert_eq!(cut.stopped, Some(WindowTextStop::Deadline));
        let fixture: WindowTextReply = serde_json::from_str(include_str!(
            "../../../contracts/fixtures/native/results/window_text.json"
        ))?;
        assert_eq!(fixture.stopped, Some(WindowTextStop::Deadline));
        Ok(())
    }
}
