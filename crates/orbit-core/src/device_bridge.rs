//! What a remote assistant may ask this Mac to do, and what it may never ask.
//!
//! Orbit can take a window screenshot, read selected text and act on
//! applications. A bridge that lets an assistant ask for the same things is
//! useful and is also the largest attack surface in the product, so the rule
//! about what it may reach is a pure decision, decided here and testable
//! without a Mac.
//!
//! REF-11: every capability answers to a permission macOS grants — Screen
//! Recording, Accessibility or Calendars — and to nothing Orbit keeps. Mars
//! grants and withdraws those in System Settings; that is the whole of her
//! decision, so no switch inside Orbit stands in front of any of them. What
//! still cuts everything is her privacy stop.

use crate::computer_control::engine::SystemPermissions;

/// What a remote request wants from the Mac.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// A picture of one window the person already has open.
    Screenshot,
    /// The list of windows that could be captured.
    ScreenSources,
    /// The text the person currently has selected.
    SelectedText,
    /// The tools the engine on this Mac declares it can run.
    ///
    /// Asked of the engine rather than answered from a list Orbit keeps, so a
    /// newer engine reaches the assistant without a change here.
    ControlTools,
    /// One of those tools, run on this Mac.
    PerformAction,
    /// The calendar, read from EventKit for the dates the caller names, now.
    ///
    /// CAL-03: not a copy of a window Orbit chose in advance. The range travels
    /// with the request, the read happens when it arrives, and nothing is kept.
    CalendarRead,
}

/// Which permission of macOS governs a capability.
///
/// One arm per kind, so adding a capability forces a decision about which
/// permission governs it instead of defaulting into whichever branch is the
/// `else`. None of them is a switch in Orbit: CAL-01 removed the calendar's,
/// DEI-16 the per-action consent and REF-11 the last three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// The screen and the computer: Screen Recording, Accessibility or both.
    Computer(SystemPermissions),
    /// Reads the calendar: the macOS calendar permission and nothing else.
    Calendar,
}

impl Capability {
    pub const ALL: [Self; 6] = [
        Self::Screenshot,
        Self::ScreenSources,
        Self::SelectedText,
        Self::ControlTools,
        Self::PerformAction,
        Self::CalendarRead,
    ];

    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Screenshot => "screenshot",
            Self::ScreenSources => "screen_sources",
            Self::SelectedText => "selected_text",
            Self::ControlTools => "control_tools",
            Self::PerformAction => "perform_action",
            Self::CalendarRead => "calendar_read",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "screenshot" => Some(Self::Screenshot),
            "screen_sources" => Some(Self::ScreenSources),
            "selected_text" => Some(Self::SelectedText),
            "control_tools" => Some(Self::ControlTools),
            "perform_action" => Some(Self::PerformAction),
            "calendar_read" => Some(Self::CalendarRead),
            _ => None,
        }
    }

    /// Which permission of macOS this capability answers to.
    ///
    /// A picture of a window and the list of windows read the screen. The
    /// selected text reads a window's identity on screen and its text through
    /// Accessibility (`AXSelectedTextCapture.swift`), and the engine — its
    /// catalogue and its actions — needs both to run at all.
    pub fn class(self) -> Class {
        match self {
            Self::Screenshot | Self::ScreenSources => Class::Computer(SystemPermissions::SCREEN),
            Self::SelectedText | Self::ControlTools | Self::PerformAction => {
                Class::Computer(SystemPermissions::ENGINE)
            }
            Self::CalendarRead => Class::Calendar,
        }
    }
}

/// The most windows named in one answer.
///
/// A cap rather than the whole list: the answer travels in a hub frame, and a
/// Mac with eighty windows open would otherwise decide the frame size.
pub const MAX_REPORTED_WINDOWS: usize = 24;

/// What a remote caller is told about the windows that are open.
///
/// Deliberately narrower than the local inventory. Window ids and process ids
/// are how the Mac addresses a window, not something an assistant needs to know,
/// and handing them out would invite a caller to compose requests out of
/// identifiers it was never given a way to obtain.
///
/// Titles do go out: "what do I have open" is unanswerable without them. What
/// lets them out is the Screen Recording permission Mars grants in macOS.
pub fn window_summary(inventory: &crate::sources::WindowInventory) -> Vec<(String, String)> {
    inventory
        .windows
        .iter()
        // A window the Mac has already marked unavailable is left out rather
        // than reported with its reason: the reason is a local diagnostic, and
        // listing something that cannot be captured only invites asking for it.
        .filter(|window| window.unavailable_reason.is_none())
        .take(MAX_REPORTED_WINDOWS)
        .map(|window| (window.application_name.clone(), window.title.clone()))
        .collect()
}

/// The most bytes one page of the engine catalogue may occupy once encoded.
///
/// Why a page at all: the whole catalogue is 147 909 bytes on the Mac it is
/// measured on, and the link to the hub refuses any outgoing frame over
/// 128 KiB. That ceiling is shared with recordings, calendar, transcripts and
/// messages and exists so one large reply cannot monopolise the link, so the
/// catalogue is what has to change, not the ceiling.
///
/// Why 96 KiB: the reply that carries a page adds its own envelope — request
/// id, method, the page's own counters — on top of the tools, and a page has to
/// clear the ceiling with that envelope included. 96 KiB leaves 32 KiB of
/// headroom for it and still splits the measured catalogue into two pages
/// instead of a dozen, which is the whole round-trip cost of this decision.
pub const MAX_CATALOGUE_PAGE_BYTES: usize = 96 * 1024;

/// What a caller is told about a tool whose own detail does not fit a page.
///
/// The name survives, because a name the assistant can read is the difference
/// between a tool it can ask about and a tool that silently does not exist. The
/// detail does not, because nothing can make it fit.
///
/// DEI-16: it names the detail, not only the schema. Since a description is no
/// longer bounded on its own, either half of what the engine wrote about a tool
/// can be the half that will not travel, and the notice has to be true of both.
const DETAIL_TOO_LARGE: &str = "Orbit no puede enviar el detalle de esta herramienta —su \
     descripción y su esquema—: no cabe en una trama del hub. El nombre sí, y con él se puede \
     pedir.";

/// One slice of the engine catalogue, and where the next one starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CataloguePage {
    /// The tools in this page, as the engine declared them.
    pub tools: Vec<crate::computer_control::EngineTool>,
    /// The index this page starts at, echoed back so a caller can tell an
    /// answer to its question from an answer to a different one.
    pub after_index: usize,
    /// Where to continue, or nothing when this page is the last one.
    pub next_after_index: Option<usize>,
    /// How many tools the whole catalogue has, so a caller that walked every
    /// page can tell a complete walk from a truncated one.
    pub total: usize,
}

/// The tool as it will travel, or its name alone when it cannot.
fn bounded_entry(
    tool: &crate::computer_control::EngineTool,
) -> crate::computer_control::EngineTool {
    if encoded_length(tool) <= MAX_CATALOGUE_PAGE_BYTES {
        return tool.clone();
    }
    crate::computer_control::EngineTool {
        name: tool.name.clone(),
        description: DETAIL_TOO_LARGE.to_owned(),
        input_schema: serde_json::Value::Null,
    }
}

/// The bytes this entry costs inside the page's array, comma included.
///
/// A tool that cannot be encoded at all is reported as unaffordable rather than
/// silently priced at zero, so it takes the same route as an oversized one.
fn encoded_length(tool: &crate::computer_control::EngineTool) -> usize {
    serde_json::to_vec(tool).map_or(usize::MAX, |bytes| bytes.len().saturating_add(1))
}

/// The slice of the catalogue that starts at `after_index` and still fits one
/// frame, plus where the next slice starts.
///
/// Pure and total: no clock, no I/O, and no input makes it panic or loop. An
/// `after_index` past the end yields the empty last page rather than an error,
/// because "nothing left" is the honest answer to it.
///
/// Every page carries at least one tool whenever tools remain. A page that
/// could come back empty with a continuation would send a caller round the
/// same index for ever without ever telling it why.
pub fn catalogue_page(
    catalogue: &[crate::computer_control::EngineTool],
    after_index: usize,
) -> CataloguePage {
    let total = catalogue.len();
    let start = after_index.min(total);
    let mut tools = Vec::new();
    let mut used = 0usize;
    let mut index = start;
    while let Some(tool) = catalogue.get(index) {
        let entry = bounded_entry(tool);
        let cost = encoded_length(&entry);
        if !tools.is_empty() && used.saturating_add(cost) > MAX_CATALOGUE_PAGE_BYTES {
            break;
        }
        used = used.saturating_add(cost);
        tools.push(entry);
        index = index.saturating_add(1);
    }
    CataloguePage {
        tools,
        after_index: start,
        next_after_index: (index < total).then_some(index),
        total,
    }
}

/// How many bytes of calendar events one bridge answer may carry.
///
/// Protects: the link to the hub, which refuses any outgoing frame over
/// 128 KiB — a ceiling shared with recordings, transcripts and messages so one
/// large reply cannot monopolise it. Why 96 KiB: the same reasoning and number
/// as `MAX_CATALOGUE_PAGE_BYTES`, leaving 32 KiB for the envelope. What it is
/// not: a limit on how much of the agenda Deimos gets. REF-10: an answer that
/// does not fit is paged, and the translator joins the pages into one reply.
pub const MAX_CALENDAR_PAGE_BYTES: usize = 96 * 1024;

/// Where a calendar answer resumes inside the events of one window: the event,
/// and inside it the first invitee not sent yet (0 for a whole event).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct CalendarPosition {
    pub index: usize,
    pub invitee: usize,
}

/// One slice of the events of a window, and where the next slice starts.
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarPage {
    pub events: Vec<serde_json::Value>,
    /// Bytes the slice costs inside the answer's array, separators included.
    pub used: usize,
    /// Where to continue in this window, or nothing when it is exhausted.
    pub next: Option<CalendarPosition>,
    /// Events that could not fit a page even alone, named (decision 4). They
    /// are passed over; the slice goes on with the next event.
    pub oversized: Vec<crate::calendar::CalendarOversizedEvent>,
}

/// The field a fragment of one event carries while more of its invitees follow.
pub const INVITEES_CONTINUED: &str = "invitees_continued";

fn encoded(value: &serde_json::Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len().saturating_add(1))
}

/// The events of `events` from `from` that fit `budget`, whole and in order.
///
/// Pure and total: no clock, no I/O, and no input makes it panic or loop. Whole
/// events only while they fit. `answer_started` says whether the answer this
/// slice joins already carries events from an earlier window: if it does and
/// nothing more fits, the slice is empty and `next` is `from`, so the answer
/// can go out; if it does not, the slice always carries something while events
/// remain, so a walk over pages ends. The one event that cannot fit a page on
/// its own is a meeting with a great many invitees, and it is not trimmed: it
/// travels in fragments, each the same event with the next part of its invitee
/// list, every fragment but the last marked `invitees_continued`. The reader
/// joins them. REF-10 A3: nothing a day holds is left out for size — except,
/// by the reviewer's decision 4, the event that cannot fit a page even alone
/// (without its guests, or with its single largest guest). That one is passed
/// over and named in `oversized`, with the field to blame, its bytes and the
/// page's; the rest of the window goes on.
pub fn calendar_page(
    events: &[crate::calendar::CalendarOccurrence],
    from: CalendarPosition,
    budget: usize,
    answer_started: bool,
) -> Result<CalendarPage, serde_json::Error> {
    let mut page = Vec::new();
    let mut used = 0_usize;
    let mut position = from;
    let mut oversized = Vec::new();
    while let Some(event) = events.get(position.index) {
        let invitees = event.invitees.as_deref().unwrap_or(&[]);
        let rest = invitees.get(position.invitee..).unwrap_or(&[]);
        let mut value = serde_json::to_value(event)?;
        if position.invitee > 0 {
            value["invitees"] = serde_json::to_value(rest)?;
        }
        let cost = encoded(&value);
        if used.saturating_add(cost) <= budget {
            used = used.saturating_add(cost);
            page.push(value);
            position = CalendarPosition {
                index: position.index.saturating_add(1),
                invitee: 0,
            };
            continue;
        }
        if !page.is_empty() || answer_started {
            return Ok(CalendarPage {
                events: page,
                used,
                next: Some(position),
                oversized,
            });
        }
        // Alone and still too large. Send as many invitees as fit, at least
        // one so the walk moves, and continue from the next — unless the event
        // cannot travel at all: then it is named and passed over.
        let mut fragment = value;
        fragment["invitees"] = serde_json::Value::Array(Vec::new());
        fragment[INVITEES_CONTINUED] = serde_json::Value::Bool(true);
        let mut size = encoded(&fragment);
        if position.invitee == 0
            && let Some(left_out) = cannot_travel(event, size, budget)?
        {
            oversized.push(left_out);
            position = CalendarPosition {
                index: position.index.saturating_add(1),
                invitee: 0,
            };
            continue;
        }
        let mut take = 0_usize;
        for invitee in rest {
            let cost = serde_json::to_vec(invitee)
                .map_or(usize::MAX, |bytes| bytes.len())
                .saturating_add(1);
            if take > 0 && size.saturating_add(cost) > budget {
                break;
            }
            size = size.saturating_add(cost);
            take = take.saturating_add(1);
        }
        let sent = position.invitee.saturating_add(take);
        fragment["invitees"] = serde_json::to_value(rest.get(..take).unwrap_or(rest))?;
        let finished = sent >= invitees.len();
        if finished && let Some(fields) = fragment.as_object_mut() {
            fields.remove(INVITEES_CONTINUED);
        }
        used = used.saturating_add(encoded(&fragment));
        page.push(fragment);
        let next = if finished {
            CalendarPosition {
                index: position.index.saturating_add(1),
                invitee: 0,
            }
        } else {
            CalendarPosition {
                index: position.index,
                invitee: sent,
            }
        };
        return Ok(CalendarPage {
            events: page,
            used,
            next: (next.index < events.len()).then_some(next),
            oversized,
        });
    }
    Ok(CalendarPage {
        events: page,
        used,
        next: None,
        oversized,
    })
}

/// Whether one event cannot travel in a page of `budget` bytes even alone:
/// without its guests (`base` bytes), or with the largest of them — fragments
/// carry at least one guest each. Names the field to blame, its bytes and the
/// page's (decision 4).
fn cannot_travel(
    event: &crate::calendar::CalendarOccurrence,
    base: usize,
    budget: usize,
) -> Result<Option<crate::calendar::CalendarOversizedEvent>, serde_json::Error> {
    let largest_guest = event
        .invitees
        .iter()
        .flatten()
        .map(|invitee| serde_json::to_vec(invitee).map(|bytes| bytes.len().saturating_add(1)))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .unwrap_or(0);
    let (field, bytes) = if base > budget {
        // The largest single field is the one to name.
        let fields = [
            ("title", event.title.len()),
            ("event_url", event.event_url.as_deref().map_or(0, str::len)),
            ("calendar_item_id", event.calendar_item_id.len()),
            ("event_id", event.event_id.as_deref().map_or(0, str::len)),
        ];
        let blamed = fields
            .iter()
            .copied()
            .max_by_key(|(_, length)| *length)
            .map_or("event", |(name, _)| name);
        (blamed, base)
    } else if base.saturating_add(largest_guest) > budget {
        ("invitees", base.saturating_add(largest_guest))
    } else {
        return Ok(None);
    };
    Ok(Some(crate::calendar::CalendarOversizedEvent {
        calendar: event.calendar.clone(),
        calendar_item_id: Some(event.calendar_item_id.clone()),
        start_unix_ms: event.start_unix_ms,
        end_unix_ms: event.end_unix_ms,
        field: field.to_owned(),
        bytes: u64::try_from(bytes).unwrap_or(u64::MAX),
        limit: u64::try_from(budget).unwrap_or(u64::MAX),
    }))
}

/// What the Mac knows of Orbit's calendar permission, in three values.
///
/// REF-10 A8: `Unknown` is not a softer `NotGranted` and not an optimistic
/// `Granted`. It is the state between the helper starting and it saying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarPermission {
    Granted,
    NotGranted,
    Unknown,
}

impl CalendarPermission {
    pub fn from_authorization(authorization: crate::calendar::CalendarAuthorization) -> Self {
        use crate::calendar::CalendarAuthorization as Authorization;
        match authorization {
            Authorization::FullAccess => Self::Granted,
            Authorization::NotDetermined
            | Authorization::Restricted
            | Authorization::Denied
            | Authorization::WriteOnly => Self::NotGranted,
            Authorization::Unavailable | Authorization::Unknown => Self::Unknown,
        }
    }
}

/// Everything the decision needs, passed in rather than read from the world.
///
/// Every permission here is what macOS says, in three values; nothing is a
/// switch of Orbit. REF-11 took the last two (`observation` and `action`) out,
/// and keeping the calendar beside the other two is what stops a future edit
/// from quietly adding an Orbit-side switch in front of any of them.
#[derive(Debug, Clone, Copy)]
pub struct Conditions {
    pub privacy_stopped: bool,
    /// Whether the helper that reports the permissions is connected.
    pub helper_connected: bool,
    /// Accessibility, as the Mac last reported it.
    pub accessibility: crate::PermissionState,
    /// Screen Recording, as the Mac last reported it.
    pub screen_recording: crate::PermissionState,
    /// What macOS grants Orbit for the calendar, as the Mac last said.
    pub calendar: CalendarPermission,
}

/// Whether this Mac will serve a remote request, and why not when it will not.
///
/// Fails closed at every step: an unrecognised capability is refused rather than
/// passed through, and a permission is required as macOS reports it rather than
/// assumed; one macOS has not reported yet is refused as unknown, never as «no».
pub fn admits(
    capability: &str,
    conditions: Conditions,
) -> Result<Capability, crate::refusal::Refusal> {
    use crate::refusal::{Expected, Parameter, Permission, Refusal};
    if conditions.privacy_stopped {
        return Err(Refusal::PrivacyStopped { since_ms: None });
    }
    let capability = Capability::parse(capability).ok_or(Refusal::InvalidParameter {
        parameter: Parameter::Request,
        expected: Expected::ValidIdentifier,
    })?;
    match capability.class() {
        Class::Computer(required) => {
            crate::computer_control::engine::admits(
                required,
                crate::computer_control::engine::Observed {
                    privacy_stopped: false,
                    helper_connected: conditions.helper_connected,
                    accessibility: conditions.accessibility,
                    screen_recording: conditions.screen_recording,
                },
            )?;
        }
        Class::Calendar if conditions.calendar == CalendarPermission::NotGranted => {
            return Err(Refusal::PermissionDenied {
                permission: Permission::Calendar,
            });
        }
        Class::Calendar if conditions.calendar == CalendarPermission::Unknown => {
            return Err(Refusal::PermissionUnknown {
                permission: Permission::Calendar,
            });
        }
        Class::Calendar => {}
    }
    Ok(capability)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PermissionState;
    use crate::refusal::Refusal;

    const COMPUTER_CAPABILITIES: [&str; 5] = [
        "screenshot",
        "screen_sources",
        "selected_text",
        "control_tools",
        "perform_action",
    ];

    const ONLY_ACCESSIBILITY: SystemPermissions = SystemPermissions {
        accessibility: true,
        screen_recording: false,
    };

    /// What macOS grants, and nothing else: there is no switch to pass in.
    fn granted(accessibility: PermissionState, screen_recording: PermissionState) -> Conditions {
        Conditions {
            privacy_stopped: false,
            helper_connected: true,
            accessibility,
            screen_recording,
            calendar: CalendarPermission::Granted,
        }
    }

    fn everything() -> Conditions {
        granted(PermissionState::Granted, PermissionState::Granted)
    }

    /// CAL-03 A7: her macOS calendar permission is the only decision.
    ///
    /// The computer permissions are refused here on purpose. If this ever
    /// goes back to refusing the calendar, the calendar has grown a second
    /// layer again — the exact shape CAL-01 removed.
    #[test]
    fn cal03_a7_the_calendar_read_answers_to_the_system_permission_alone() {
        let calendar_only = Conditions {
            calendar: CalendarPermission::Granted,
            ..granted(PermissionState::Denied, PermissionState::Denied)
        };
        assert_eq!(
            admits("calendar_read", calendar_only),
            Ok(Capability::CalendarRead)
        );
        assert_eq!(Capability::CalendarRead.class(), Class::Calendar);
    }

    /// CAL-03 A6: withdrawing the permission cuts it off, and says why.
    ///
    /// Every other permission is granted here, so the only thing refusing is
    /// the calendar's.
    #[test]
    fn cal03_a6_without_the_system_permission_the_calendar_is_refused_by_name() {
        let revoked = Conditions {
            calendar: CalendarPermission::NotGranted,
            ..everything()
        };
        assert_eq!(
            admits("calendar_read", revoked),
            Err(Refusal::PermissionDenied {
                permission: crate::refusal::Permission::Calendar
            })
        );
        assert_eq!(
            Refusal::PermissionDenied {
                permission: crate::refusal::Permission::Calendar
            }
            .code()
            .name(),
            "permission_denied"
        );
        // Not an empty answer, and not a bare code: it names the permission and
        // where to grant it, because "nothing" reads as "your diary is empty".
    }

    /// The calendar permission does not buy its way past the other rules.
    #[test]
    fn the_calendar_read_obeys_privacy() {
        assert_eq!(
            admits(
                "calendar_read",
                Conditions {
                    privacy_stopped: true,
                    ..everything()
                }
            ),
            Err(Refusal::PrivacyStopped { since_ms: None })
        );
    }

    /// Granting the calendar does not open the screen, and the screen and the
    /// computer do not open the calendar.
    #[test]
    fn cal03_the_calendar_does_not_leak_into_the_computer_or_back() {
        let calendar_only = granted(PermissionState::Denied, PermissionState::Denied);
        assert_eq!(
            admits("screenshot", calendar_only),
            Err(Refusal::PermissionDenied {
                permission: crate::refusal::missing_permission_name(SystemPermissions::SCREEN)
            })
        );
        assert_eq!(
            admits("perform_action", calendar_only),
            Err(Refusal::PermissionDenied {
                permission: crate::refusal::missing_permission_name(SystemPermissions::ENGINE)
            })
        );
        let screens_only = Conditions {
            calendar: CalendarPermission::NotGranted,
            ..everything()
        };
        assert_eq!(
            admits("screenshot", screens_only),
            Ok(Capability::Screenshot)
        );
        assert_eq!(
            admits("calendar_read", screens_only),
            Err(Refusal::PermissionDenied {
                permission: crate::refusal::Permission::Calendar
            })
        );
    }

    /// REF-11 A6: with the permissions granted, every capability is served,
    /// and the conditions carry no setting of Orbit to consult.
    #[test]
    fn ref11_a6_every_capability_is_served_with_the_permissions_and_no_switch() {
        for (name, capability) in [
            ("screenshot", Capability::Screenshot),
            ("screen_sources", Capability::ScreenSources),
            ("selected_text", Capability::SelectedText),
            ("control_tools", Capability::ControlTools),
            ("perform_action", Capability::PerformAction),
        ] {
            assert_eq!(
                admits(name, everything()),
                Ok(capability),
                "{name} se negó con los permisos concedidos"
            );
        }
        assert_eq!(
            admits("calendar_read", everything()),
            Ok(Capability::CalendarRead)
        );
    }

    /// REF-11 A3: a refusal names the permission that is missing, and one the
    /// Mac has not reported is its own refusal, never a «no».
    #[test]
    fn ref11_a3_a_refusal_names_which_permission_and_unknown_is_not_no() {
        let no_accessibility = granted(PermissionState::Denied, PermissionState::Granted);
        assert_eq!(
            admits("perform_action", no_accessibility),
            Err(Refusal::PermissionDenied {
                permission: crate::refusal::missing_permission_name(ONLY_ACCESSIBILITY)
            })
        );
        assert_eq!(
            Refusal::PermissionDenied {
                permission: crate::refusal::missing_permission_name(ONLY_ACCESSIBILITY)
            }
            .code()
            .name(),
            "permission_denied"
        );

        let not_granted = granted(PermissionState::NotGranted, PermissionState::Granted);
        let typed = admits("perform_action", not_granted);
        assert_eq!(
            typed,
            Err(crate::refusal::Refusal::PermissionNotGranted {
                permission: crate::refusal::Permission::Accessibility,
            })
        );
        let Err(cause) = typed else {
            panic!("a non-grant must refuse")
        };
        let wire = serde_json::to_value(&cause)
            .unwrap_or_else(|error| panic!("synthetic refusal: {error}"));
        assert_eq!(wire["code"], "permission_not_granted");

        let not_reported = granted(PermissionState::Unknown, PermissionState::Unknown);
        for capability in COMPUTER_CAPABILITIES {
            let refused = admits(capability, not_reported);
            assert!(
                matches!(refused, Err(Refusal::PermissionUnknown { .. })),
                "{capability}: {refused:?}"
            );
            if let Err(refusal) = refused {
                assert_eq!(refusal.code().name(), "permission_unknown");
            }
        }
    }

    fn occurrence(index: usize, invitees: usize) -> crate::calendar::CalendarOccurrence {
        use crate::calendar::{CalendarFieldStatus, CalendarInvitee, CalendarReference};
        crate::calendar::CalendarOccurrence {
            observation_id: format!("00000000-0000-4000-8000-{index:012}"),
            calendar: CalendarReference {
                calendar_id: "work".into(),
                source_id: "account".into(),
            },
            calendar_item_id: format!("item-{index}"),
            event_id: None,
            original_occurrence_unix_ms: None,
            start_unix_ms: 1_790_805_600_000 + i64::try_from(index).unwrap_or(0) * 60_000,
            end_unix_ms: 1_790_809_200_000 + i64::try_from(index).unwrap_or(0) * 60_000,
            event_timezone: None,
            interpreted_timezone: "Europe/Madrid".into(),
            is_floating: true,
            is_all_day: false,
            is_detached: false,
            title: format!("Evento {index}"),
            status: "confirmed".into(),
            last_modified_unix_ms: None,
            invitees: Some(
                (0..invitees)
                    .map(|guest| CalendarInvitee {
                        name: Some(format!("Persona {guest}")),
                        participant_url: format!("mailto:persona{guest}@example.invalid"),
                        email: Some(format!("persona{guest}@example.invalid")),
                        role: "required".into(),
                        response_status: "accepted".into(),
                        participant_type: "person".into(),
                        is_current_user: false,
                    })
                    .collect(),
            ),
            invitees_status: CalendarFieldStatus::Provided,
            event_url: None,
            event_url_status: CalendarFieldStatus::NotProvided,
            location: None,
            notes: None,
            notes_left_out: None,
            organizer: None,
            availability: None,
            recurrence: None,
            can_edit: None,
            alarms_minutes: None,
            alarms_not_editable: None,
        }
    }

    /// REF-10 A3: whole events while they fit, and a page that stops says
    /// exactly where the next one starts. Nothing is counted as left out.
    #[test]
    fn ref10_a3_a_page_takes_whole_events_and_says_where_to_go_on() -> Result<(), serde_json::Error>
    {
        let events: Vec<_> = (0..50).map(|index| occurrence(index, 2)).collect();
        let everything = calendar_page(
            &events,
            CalendarPosition::default(),
            MAX_CALENDAR_PAGE_BYTES,
            false,
        )?;
        assert_eq!(
            everything.events.len(),
            50,
            "todo cabe de sobra bajo el techo"
        );
        assert_eq!(everything.next, None);

        let one = serde_json::to_vec(&events[0])?.len() + 1;
        let three = calendar_page(&events, CalendarPosition::default(), one * 3, false)?;
        assert_eq!(three.events.len(), 3);
        assert_eq!(
            three.next,
            Some(CalendarPosition {
                index: 3,
                invitee: 0
            })
        );
        // Walking the pages yields every event exactly once, in order.
        let mut walked = Vec::new();
        let mut position = Some(CalendarPosition::default());
        while let Some(from) = position {
            let page = calendar_page(&events, from, one * 3, false)?;
            assert!(
                !page.events.is_empty(),
                "a page that continues carries something"
            );
            walked.extend(page.events);
            position = page.next;
        }
        assert_eq!(walked.len(), events.len());
        assert_eq!(walked[49]["title"], "Evento 49");
        // Joining an answer that already carries events: nothing is forced in.
        let full = calendar_page(&events, CalendarPosition::default(), one - 1, true)?;
        assert!(full.events.is_empty());
        assert_eq!(full.next, Some(CalendarPosition::default()));
        let full_day: Vec<_> = (0..400).map(|index| occurrence(index, 2)).collect();
        let filled = calendar_page(
            &full_day,
            CalendarPosition::default(),
            MAX_CALENDAR_PAGE_BYTES,
            false,
        )?;
        assert!(filled.used > MAX_CALENDAR_PAGE_BYTES * 9 / 10);
        assert!(filled.used <= MAX_CALENDAR_PAGE_BYTES);
        assert!(filled.next.is_some());
        const { assert!(MAX_CALENDAR_PAGE_BYTES < 128 * 1024) };
        Ok(())
    }

    /// REF-10 A3: a meeting whose own invitees do not fit one frame travels in
    /// fragments, each under the budget, and no invitee is dropped.
    #[test]
    fn ref10_a3_one_event_larger_than_a_page_travels_in_fragments() -> Result<(), serde_json::Error>
    {
        let events = vec![occurrence(0, 2_000), occurrence(1, 1)];
        let mut fragments = Vec::new();
        let mut position = Some(CalendarPosition::default());
        let mut pages = 0;
        while let Some(from) = position {
            let page = calendar_page(&events, from, MAX_CALENDAR_PAGE_BYTES, false)?;
            assert!(page.used <= MAX_CALENDAR_PAGE_BYTES, "{} bytes", page.used);
            fragments.extend(page.events);
            position = page.next;
            pages += 1;
        }
        assert!(pages > 2, "two thousand invitees need more than one frame");
        let (split, rest): (Vec<_>, Vec<_>) = fragments
            .into_iter()
            .partition(|value| value["observation_id"] == events[0].observation_id.as_str());
        let joined: usize = split
            .iter()
            .map(|value| value["invitees"].as_array().map_or(0, Vec::len))
            .sum();
        assert_eq!(joined, 2_000);
        // Every fragment but the last says more follow; the last says nothing.
        assert!(split.len() > 1, "the meeting travelled in fragments");
        let last = split.len() - 1;
        assert!(
            split[..last]
                .iter()
                .all(|value| value[INVITEES_CONTINUED] == true)
        );
        assert!(split[last].get(INVITEES_CONTINUED).is_none());
        assert_eq!(rest.len(), 1);
        Ok(())
    }

    /// The calendar answers to its own permission even with no helper.
    #[test]
    fn the_calendar_permission_is_independent_of_helper_presence() {
        let no_helper = Conditions {
            helper_connected: false,
            ..everything()
        };
        assert_eq!(
            Refusal::HelperNotRunning.code().name(),
            "helper_not_running"
        );

        assert_eq!(
            admits("calendar_read", no_helper),
            Ok(Capability::CalendarRead)
        );
    }

    /// REF-10 A8: a permission the Mac has not reported yet is neither granted
    /// nor denied, and the refusal says which of the three it is.
    #[test]
    fn ref10_a8_an_unknown_calendar_permission_is_its_own_refusal() {
        use crate::calendar::CalendarAuthorization as Authorization;
        let conditions = |calendar| Conditions {
            calendar,
            ..granted(PermissionState::Denied, PermissionState::Denied)
        };
        assert_eq!(
            admits("calendar_read", conditions(CalendarPermission::Unknown)),
            Err(Refusal::PermissionUnknown {
                permission: crate::refusal::Permission::Calendar
            })
        );
        assert_eq!(
            Refusal::PermissionUnknown {
                permission: crate::refusal::Permission::Calendar
            }
            .code()
            .name(),
            "permission_unknown"
        );

        for (authorization, expected) in [
            (Authorization::FullAccess, CalendarPermission::Granted),
            (Authorization::Denied, CalendarPermission::NotGranted),
            (Authorization::NotDetermined, CalendarPermission::NotGranted),
            (Authorization::WriteOnly, CalendarPermission::NotGranted),
            (Authorization::Restricted, CalendarPermission::NotGranted),
            (Authorization::Unknown, CalendarPermission::Unknown),
            (Authorization::Unavailable, CalendarPermission::Unknown),
        ] {
            assert_eq!(
                CalendarPermission::from_authorization(authorization),
                expected
            );
        }
    }

    #[test]
    fn privacy_stopped_refuses_everything_including_merely_looking() {
        // The latch is the one control that means the same thing to every
        // capability; carving out an exception would make it unreadable.
        let stopped = Conditions {
            privacy_stopped: true,
            ..everything()
        };
        for capability in COMPUTER_CAPABILITIES {
            assert_eq!(
                admits(capability, stopped),
                Err(Refusal::PrivacyStopped { since_ms: None })
            );
        }
    }

    /// Screen Recording lets it look; acting needs Accessibility as well,
    /// because that is what macOS itself asks of anything that clicks.
    #[test]
    fn screen_recording_alone_lets_it_look_but_not_act() {
        let looking_only = granted(PermissionState::Denied, PermissionState::Granted);
        assert_eq!(
            admits("screen_sources", looking_only),
            Ok(Capability::ScreenSources)
        );
        assert_eq!(
            admits("screenshot", looking_only),
            Ok(Capability::Screenshot)
        );
        for capability in ["selected_text", "control_tools", "perform_action"] {
            assert_eq!(
                admits(capability, looking_only),
                Err(Refusal::PermissionDenied {
                    permission: crate::refusal::missing_permission_name(ONLY_ACCESSIBILITY)
                }),
                "{capability}"
            );
        }
    }

    #[test]
    fn an_unknown_capability_is_refused_rather_than_forwarded() {
        assert_eq!(
            admits("rm_minus_rf", everything()),
            Err(Refusal::InvalidParameter {
                parameter: crate::refusal::Parameter::Request,
                expected: crate::refusal::Expected::ValidIdentifier
            })
        );
    }

    #[test]
    fn selected_text_needs_the_screen_and_accessibility() {
        // The helper reads the window's identity on screen and its text through
        // Accessibility, so either one missing refuses it by name.
        assert_eq!(
            admits(
                "selected_text",
                granted(PermissionState::Granted, PermissionState::Denied)
            ),
            Err(Refusal::PermissionDenied {
                permission: crate::refusal::missing_permission_name(SystemPermissions::SCREEN)
            })
        );
        assert_eq!(
            admits(
                "selected_text",
                granted(PermissionState::Denied, PermissionState::Granted)
            ),
            Err(Refusal::PermissionDenied {
                permission: crate::refusal::missing_permission_name(ONLY_ACCESSIBILITY)
            })
        );
        assert_eq!(
            admits("selected_text", everything()),
            Ok(Capability::SelectedText)
        );
        let stopped = Conditions {
            privacy_stopped: true,
            ..everything()
        };
        assert_eq!(
            admits("selected_text", stopped),
            Err(Refusal::PrivacyStopped { since_ms: None })
        );
    }

    #[test]
    fn dei07_a1_rejections_keep_their_security_precedence() {
        let stopped_without_permissions = Conditions {
            privacy_stopped: true,
            calendar: CalendarPermission::NotGranted,
            ..granted(PermissionState::Denied, PermissionState::Denied)
        };
        assert_eq!(
            admits("inventado", stopped_without_permissions),
            Err(Refusal::PrivacyStopped { since_ms: None })
        );

        let without_permissions = Conditions {
            privacy_stopped: false,
            ..stopped_without_permissions
        };
        assert_eq!(
            admits("perform_action", without_permissions),
            Err(Refusal::PermissionDenied {
                permission: crate::refusal::missing_permission_name(SystemPermissions::ENGINE)
            })
        );
        assert_eq!(
            admits("control_tools", without_permissions),
            Err(Refusal::PermissionDenied {
                permission: crate::refusal::missing_permission_name(SystemPermissions::ENGINE)
            })
        );
    }

    fn window(
        name: &str,
        title: &str,
        unavailable: Option<&str>,
    ) -> crate::sources::WindowCandidate {
        crate::sources::WindowCandidate {
            binding: None,
            window_id: 7,
            application_pid: 4242,
            application_name: name.into(),
            bundle_id: "com.ejemplo.app".into(),
            title: title.into(),
            title_truncated: false,
            unavailable_reason: unavailable.map(str::to_owned),
        }
    }

    fn inventory(windows: Vec<crate::sources::WindowCandidate>) -> crate::sources::WindowInventory {
        crate::sources::WindowInventory {
            native_instance_id: "instancia".into(),
            privacy_generation: 1,
            inventory_id: "inventario".into(),
            clock_domain: "monotonic".into(),
            observed_monotonic_ns: 10,
            expires_monotonic_ns: 20,
            selected_text_available: false,
            windows,
            omitted_count: 0,
        }
    }

    #[test]
    fn the_answer_names_applications_and_titles_and_nothing_else() {
        // Window ids and process ids are how the Mac addresses a window. Handing
        // them out would invite composing requests from identifiers the caller
        // was never given a way to obtain.
        let summary = window_summary(&inventory(vec![window("Safari", "Correo", None)]));
        assert_eq!(summary, vec![("Safari".to_owned(), "Correo".to_owned())]);
    }

    #[test]
    fn a_window_the_mac_already_called_unavailable_is_left_out() {
        // Listing something that cannot be captured only invites asking for it.
        let summary = window_summary(&inventory(vec![
            window("Safari", "Correo", None),
            window("Ajustes", "Privacidad", Some("not_capturable")),
        ]));
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].0, "Safari");
    }

    #[test]
    fn a_mac_with_many_windows_does_not_decide_the_frame_size() {
        let many = (0..MAX_REPORTED_WINDOWS + 10)
            .map(|index| window("App", &format!("Ventana {index}"), None))
            .collect();
        assert_eq!(window_summary(&inventory(many)).len(), MAX_REPORTED_WINDOWS);
    }

    /// The order of magnitude the real engine publishes, measured on Mars's Mac
    /// on 2026-09-23: 147 909 bytes of tools and input schemas. Every earlier
    /// test of this path used a toy catalogue of a few hundred bytes, which is
    /// exactly why the size reached the Mac twice.
    const REAL_CATALOGUE_BYTES: usize = 147_909;

    /// The link's own ceiling, restated here rather than imported: `orbit-core`
    /// does not depend on the daemon, and the point of the page budget is that
    /// it clears this number with its envelope on top.
    const HUB_FRAME_CEILING: usize = 128 * 1024;

    fn realistic_tool(index: usize) -> crate::computer_control::EngineTool {
        let mut properties = serde_json::Map::new();
        for field in 0..10 {
            properties.insert(
                format!("parameter_{field}"),
                serde_json::json!({
                    "type": "string",
                    "title": format!("Parámetro {field}"),
                    "description": format!(
                        "Argumento {field} de la herramienta {index}, descrito por el motor con \
                         el detalle que necesita quien la llama para no adivinar su forma."
                    ),
                    "enum": ["ninguno", "primero", "segundo", "tercero"],
                }),
            );
        }
        crate::computer_control::EngineTool {
            name: format!("engine_tool_{index:03}"),
            description: format!(
                "Herramienta {index} del motor de control, con su descripción completa tal como \
                 el motor la publica en su catálogo."
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": properties,
                "required": ["parameter_0", "parameter_1"],
                "additionalProperties": false,
            }),
        }
    }

    /// A catalogue of at least `target` bytes, grown one realistic tool at a
    /// time, the way `catalogue_of` builds one for the daemon's own cases.
    fn catalogue_of(target: usize) -> (Vec<crate::computer_control::EngineTool>, usize) {
        let mut tools = Vec::new();
        let mut bytes = 2;
        let mut index = 0;
        while bytes < target {
            tools.push(realistic_tool(index));
            bytes = serde_json::to_vec(&tools).map_or(usize::MAX, |encoded| encoded.len());
            index += 1;
        }
        (tools, bytes)
    }

    /// Walks every page the way the translator does, and hands back what it
    /// assembled plus the size of the largest page it saw.
    fn walk(
        catalogue: &[crate::computer_control::EngineTool],
    ) -> (Vec<crate::computer_control::EngineTool>, usize, usize) {
        let mut assembled = Vec::new();
        let mut after = 0usize;
        let mut widest = 0usize;
        let mut rounds = 0usize;
        loop {
            let page = catalogue_page(catalogue, after);
            assert_eq!(page.after_index, after, "una página contesta a su índice");
            assert_eq!(page.total, catalogue.len());
            let encoded = serde_json::to_vec(&page.tools).map_or(usize::MAX, |bytes| bytes.len());
            widest = widest.max(encoded);
            assembled.extend(page.tools);
            rounds += 1;
            match page.next_after_index {
                Some(next) => {
                    assert!(next > after, "el recorrido avanza y no se repite");
                    after = next;
                }
                None => return (assembled, widest, rounds),
            }
        }
    }

    #[test]
    fn dei14_a1_a_real_sized_catalogue_crosses_in_pages_that_fit_the_hub_frame() {
        let (catalogue, bytes) = catalogue_of(150 * 1024);
        assert!(
            (REAL_CATALOGUE_BYTES..=REAL_CATALOGUE_BYTES * 2).contains(&bytes),
            "la prueba usa un catálogo del orden del real ({REAL_CATALOGUE_BYTES} bytes), no uno de juguete: {bytes} bytes"
        );
        assert!(
            bytes > HUB_FRAME_CEILING,
            "el catálogo entero no cabe en una trama: por eso se pagina"
        );

        let (assembled, widest, rounds) = walk(&catalogue);
        assert_eq!(assembled, catalogue, "el catálogo llega entero y en orden");
        assert!(
            widest <= MAX_CATALOGUE_PAGE_BYTES,
            "ninguna página rebasa su presupuesto: {widest} bytes"
        );
        assert!(
            widest + 1024 < HUB_FRAME_CEILING,
            "una página deja sitio de sobra para el sobre de la respuesta: {widest} bytes"
        );
        assert_eq!(
            rounds, 2,
            "un catálogo real cuesta dos vueltas, no una docena"
        );
    }

    #[test]
    fn dei14_a1_a_catalogue_that_already_fits_still_travels_in_one_page() {
        let (catalogue, _) = catalogue_of(4 * 1024);
        let page = catalogue_page(&catalogue, 0);
        assert_eq!(page.after_index, 0);
        assert_eq!(page.total, catalogue.len());
        assert_eq!(page.next_after_index, None);
        assert_eq!(page.tools, catalogue);
    }

    #[test]
    fn dei14_r2_an_empty_catalogue_is_one_empty_final_page() {
        let page = catalogue_page(&[], 0);
        assert!(page.tools.is_empty());
        assert_eq!(page.total, 0);
        assert_eq!(page.next_after_index, None);
    }

    #[test]
    fn dei14_r2_an_index_past_the_end_says_nothing_is_left_rather_than_failing() {
        let (catalogue, _) = catalogue_of(4 * 1024);
        let page = catalogue_page(&catalogue, catalogue.len() + 99);
        assert!(page.tools.is_empty());
        assert_eq!(page.after_index, catalogue.len());
        assert_eq!(page.next_after_index, None);
        assert_eq!(page.total, catalogue.len());
    }

    #[test]
    fn dei14_r2_a_tool_too_large_for_a_page_keeps_its_name_and_loses_its_schema() {
        // Nothing can make this schema fit, so the choice is between a name the
        // assistant can still ask about and a tool that silently vanishes.
        let catalogue = vec![
            realistic_tool(0),
            crate::computer_control::EngineTool {
                name: "engine_tool_enorme".into(),
                description: "Una herramienta cuyo esquema no cabe en ninguna trama.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "description": "x".repeat(MAX_CATALOGUE_PAGE_BYTES + 1),
                }),
            },
            realistic_tool(1),
        ];
        let (assembled, widest, _) = walk(&catalogue);
        assert_eq!(assembled.len(), 3, "no se pierde ninguna herramienta");
        assert_eq!(assembled[1].name, "engine_tool_enorme");
        assert!(assembled[1].input_schema.is_null());
        assert_eq!(assembled[1].description, DETAIL_TOO_LARGE);
        assert_eq!(assembled[0], catalogue[0], "las demás llegan intactas");
        assert_eq!(assembled[2], catalogue[2]);
        assert!(widest <= MAX_CATALOGUE_PAGE_BYTES);
    }

    /// DEI-16 A6: with the description no longer bounded on its own, a
    /// description can be the half that will not travel. It takes the same
    /// route DEI-14 settled for a schema — the name survives, and the notice
    /// says which tool it is and why its detail is missing — instead of the
    /// whole tool disappearing without anyone being told.
    #[test]
    fn dei16_a6_a_description_too_large_for_a_page_keeps_its_name_and_says_why() {
        let catalogue = vec![
            realistic_tool(0),
            crate::computer_control::EngineTool {
                name: "engine_tool_muy_descrita".into(),
                description: "á".repeat(MAX_CATALOGUE_PAGE_BYTES),
                input_schema: serde_json::json!({"type": "object"}),
            },
            realistic_tool(1),
        ];
        let (assembled, widest, _) = walk(&catalogue);
        assert_eq!(assembled.len(), 3, "no se pierde ninguna herramienta");
        assert_eq!(assembled[1].name, "engine_tool_muy_descrita");
        assert_eq!(assembled[1].description, DETAIL_TOO_LARGE);
        assert!(
            DETAIL_TOO_LARGE.contains("descripción") && DETAIL_TOO_LARGE.contains("esquema"),
            "el aviso nombra lo que no cabe, no sólo una de las dos mitades"
        );
        assert_eq!(assembled[0], catalogue[0], "las demás llegan intactas");
        assert_eq!(assembled[2], catalogue[2]);
        assert!(widest <= MAX_CATALOGUE_PAGE_BYTES);
    }

    /// DEI-16 A6/A7: a description merely longer than the old 4 KiB bound is
    /// not a problem at all. It travels whole, and the count is unchanged.
    #[test]
    fn dei16_a6_a_description_over_the_old_bound_travels_untouched() {
        let verbose = crate::computer_control::EngineTool {
            name: "engine_tool_descrita_con_detalle".into(),
            description: "á".repeat(5_000),
            input_schema: serde_json::json!({"type": "object"}),
        };
        let catalogue = vec![realistic_tool(0), verbose.clone()];
        let (assembled, _, rounds) = walk(&catalogue);
        assert_eq!(rounds, 1);
        assert_eq!(assembled, catalogue);
        assert_eq!(catalogue_page(&catalogue, 0).total, 2);
    }

    #[test]
    fn dei14_r2_the_worst_catalogue_the_contract_allows_still_pages_within_budget() {
        let worst: Vec<crate::computer_control::EngineTool> = (0
            ..crate::computer_control::MAX_ENGINE_TOOLS)
            .map(|index| crate::computer_control::EngineTool {
                name: format!(
                    "{index:0>width$}",
                    width = crate::computer_control::MAX_TOOL_NAME_BYTES
                ),
                description: "x".repeat(4096),
                input_schema: serde_json::json!({"type": "object"}),
            })
            .collect();
        let (assembled, widest, rounds) = walk(&worst);
        assert_eq!(assembled, worst);
        assert!(widest <= MAX_CATALOGUE_PAGE_BYTES);
        assert!(rounds >= 2, "un catálogo así no cabe en una sola vuelta");
    }
}
