//! The assistant and its chat: a turn's progress, the approvals the assistant asked for,
//! the conversations and the questions asked during a meeting.

macro_rules! assistant_notices {
    ([$next:ident $(, $rest:ident)*] ; $($entries:tt)*) => {
        $next! { [$($rest),*] ; $($entries)*
            AssistantThinking = 48 => Silent,
            AssistantResponding = 49 => Silent,
            AssistantProgress = 50 => Silent,
            AssistantAnswered = 51 => Silent,
            AssistantStaleQuestion = 52 => Silent,
            AssistantInterruptRequested = 53 => Silent,
            AssistantFailed = 54 => Failed,
            /// This device no longer owns the open conversation; start a new one.
            AssistantSessionNotFound = 55 => Fixable,
            AssistantTurnLostByRestart = 56 => Fixable,
            AssistantTurnLostByDisconnect = 57 => Fixable,
            /// The hub does not know whether Deimos answered: ask again if needed.
            AssistantOutcomeUnknown = 58 => Fixable,
            /// The interrupt did not reach the hub: interrupt again.
            AssistantInterruptNotDelivered = 59 => Fixable,
            AssistantInterruptUnconfirmed = 60 => Failed,
            /// The hub refused the interrupt (`code`): interrupt again.
            AssistantInterruptRefused = 61 => Fixable,
            /// A newer hub published a turn state this build does not know: update.
            AssistantStateUnknown = 62 => Fixable,
            ApprovalSent = 63 => Silent,
            ApprovalApplied = 64 => Silent,
            /// The approval had expired; nothing was applied.
            ApprovalExpired = 65 => Fixable,
            ApprovalOutcomeUnknown = 66 => Failed,
            ApprovalReceiptStale = 67 => Silent,
            ApprovalsRecovered = 68 => Silent,
            SessionOpened = 109 => Silent,
            /// The conversations are read from the hub, and there is no
            /// connection with it.
            ConversationsNeedHub = 110 => Fixable,
            /// A question to Deimos about the meeting was not sent (`cause`).
            MeetingQuestionNotSent = 113 => Failed,
            /// A second question while Deimos still answers the first: it was not
            /// sent, and the user can ask it again when the answer arrives.
            MeetingQuestionStillAnswering = 114 => Fixable,
        }
    };
}
pub(super) use assistant_notices;
