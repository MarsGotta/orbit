//! The status item's look: the brand's marks and the time beside them.
use super::TrayState;
use tauri::image::Image;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Appearance {
    #[default]
    Dark,
    Light,
}

impl From<tauri::Theme> for Appearance {
    fn from(theme: tauri::Theme) -> Self {
        match theme {
            tauri::Theme::Light => Self::Light,
            _ => Self::Dark,
        }
    }
}

/// The brand's menu bar images (`icons/menu-bar`, from the brand pack's
/// `3-barra-de-menus`), embedded in the binary. Each is the pack's `@2x`
/// file, 36 px: the status item draws its image 18 points high
/// (tray-icon 0.24, `set_icon_for_ns_status_item_button`), sharp at the
/// Mac's 2× density. Tauri takes one bitmap per icon, so the pack's 18 px
/// files have nowhere to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MarkImage {
    /// The small symbol in black on clear: macOS tints it as the bar's other
    /// marks, in light and dark.
    Template,
    /// The moon in red. With colour it cannot be a template: one file for a
    /// dark bar, one for a light bar.
    RecordingOnDark,
    RecordingOnLight,
    /// The moon in amber.
    NeedsYouOnDark,
    NeedsYouOnLight,
}

impl MarkImage {
    pub(super) fn is_template(self) -> bool {
        self == Self::Template
    }

    pub(super) fn pixels(self) -> Image<'static> {
        match self {
            Self::Template => tauri::include_image!("icons/menu-bar/orbitTemplate@2x.png"),
            Self::RecordingOnDark => {
                tauri::include_image!("icons/menu-bar/orbit-grabando-barra-oscuro@2x.png")
            }
            Self::RecordingOnLight => {
                tauri::include_image!("icons/menu-bar/orbit-grabando-barra-claro@2x.png")
            }
            Self::NeedsYouOnDark => {
                tauri::include_image!("icons/menu-bar/orbit-aviso-barra-oscuro@2x.png")
            }
            Self::NeedsYouOnLight => {
                tauri::include_image!("icons/menu-bar/orbit-aviso-barra-claro@2x.png")
            }
        }
    }
}

/// The status item: its image, and the text beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Look {
    pub(super) image: MarkImage,
    /// Only the time recorded, while it records; empty otherwise.
    pub(super) title: String,
}

/// «00:42», «12:04», «1:02:03».
pub(super) fn elapsed_text(ms: i64) -> String {
    let seconds = ms.max(0) / 1_000;
    let (hours, minutes, seconds) = (seconds / 3_600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// The brand manual's three marks. A recording goes first: while it
/// records, something listens, and the time beside the mark says so. An
/// assistant that waits keeps its amber moon until the user answers, and
/// shows it again when a recording ends first.
pub(super) fn look(state: &TrayState, now_ms: i64) -> Look {
    let dark = state.appearance == Appearance::Dark;
    if let Some(clock) = &state.recording {
        return Look {
            image: if dark {
                MarkImage::RecordingOnDark
            } else {
                MarkImage::RecordingOnLight
            },
            title: clock
                .recorded_ms(now_ms)
                .map(elapsed_text)
                .unwrap_or_default(),
        };
    }
    let image = match (state.needs_you, dark) {
        (false, _) => MarkImage::Template,
        (true, true) => MarkImage::NeedsYouOnDark,
        (true, false) => MarkImage::NeedsYouOnLight,
    };
    Look {
        image,
        title: String::new(),
    }
}
