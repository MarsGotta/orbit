//! Presentation facts supplied by the main webview; never meeting or conversation content.
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuState {
    pub record: bool,
    pub finish: bool,
    pub new_conversation: bool,
    pub find: bool,
    pub stop: bool,
    pub sidebar: bool,
    pub bigger: bool,
    pub smaller: bool,
    pub actual_size: bool,
    pub recover_template: bool,
}

impl Default for MenuState {
    fn default() -> Self {
        Self {
            record: false,
            finish: false,
            new_conversation: false,
            find: false,
            stop: false,
            sidebar: true,
            bigger: true,
            smaller: false,
            actual_size: false,
            recover_template: false,
        }
    }
}

#[derive(Default)]
pub struct MenuContext {
    pub(super) state: Mutex<MenuState>,
}

pub struct Command {
    pub id: &'static str,
    pub key: &'static str,
    pub enabled: bool,
    pub accelerator: &'static str,
}

/// The commands consumed by the native menu builder and its event handler.
pub fn commands(state: &MenuState) -> Vec<Command> {
    let item = |id, key, enabled, accelerator| Command {
        id,
        key,
        enabled,
        accelerator,
    };
    vec![
        item("settings", "settings", true, "Cmd+,"),
        item("record", "record", state.record, ""),
        item("finish", "finish", state.finish, ""),
        item(
            "new_conversation",
            "new_conversation",
            state.new_conversation,
            "Cmd+N",
        ),
        item("find", "find", state.find, "Cmd+F"),
        item("stop", "stop", state.stop, ""),
        item(
            "sidebar",
            if state.sidebar {
                "hide_sidebar"
            } else {
                "show_sidebar"
            },
            true,
            "Ctrl+Cmd+S",
        ),
        // Muda maps NumpadAdd to AppKit’s "+" key equivalent (its parser has no Plus token).
        item("bigger", "bigger", state.bigger, "Cmd+NumpadAdd"),
        item("smaller", "smaller", state.smaller, "Cmd+-"),
        item("actual_size", "actual_size", state.actual_size, "Cmd+0"),
        item(
            "recover_template",
            "recover_template",
            state.recover_template,
            "",
        ),
    ]
}

/// Native responder actions and application commands stay distinct in the menu
/// description, so adapters and tests never need focus to choose an Undo item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditItem {
    NativeUndo,
    NativeRedo,
    NativeCut,
    NativeCopy,
    NativePaste,
    NativeSelectAll,
    Separator,
    Command(&'static str),
}

/// The native builder consumes this description; it has no focus-dependent state.
pub fn edit_items() -> &'static [EditItem] {
    use EditItem::*;
    &[
        NativeUndo,
        NativeRedo,
        Separator,
        Command("recover_template"),
        Separator,
        NativeCut,
        NativeCopy,
        NativePaste,
        NativeSelectAll,
        Separator,
        Command("find"),
    ]
}
