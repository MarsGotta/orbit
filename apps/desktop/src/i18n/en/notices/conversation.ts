import { openSettings, worded, type Entries, NoticeData } from "./types";

const asked: Record<string, string> = {
  screenshot_full: "a screenshot of the screen",
  screenshot_window: "a screenshot of a window",
  screen_sources: "the window list",
  selected_text: "your selected text",
  control_tools: "the actions available on the Mac",
  perform_action: "to perform an action on the Mac",
  calendar_read: "your agenda",
};

function whatAssistantAsked(data: NoticeData, assistantName: string): string {
  const thing = typeof data.code === "string" ? asked[data.code] : undefined;
  return thing
    ? `${assistantName} requested ${thing}.`
    : `${assistantName} requested something from this Mac.`;
}

/** Voice notes, the assistant, calls with Deimos and what Deimos asks of the Mac. */
const base = {
  voice_note_empty: worded("fixable", "No audio was recorded. Check your microphone."),
  voice_transcribing: worded("working", "Transcribing your voice…"),
  voice_not_recognized: worded("fixable", "No speech was heard. Try again."),
  voice_transcription_failed: worded(
    "fixable",
    "Couldn’t transcribe your voice note. It’s still saved in the conversation.",
  ),
  assistant_session_not_found: worded(
    "fixable",
    "That conversation isn’t on this Mac. Start a new one.",
  ),
  assistant_turn_lost_by_restart: worded(
    "fixable",
    "Your last question went unanswered after a restart. Ask again.",
  ),

  assistant_interrupt_not_delivered: worded(
    "fixable",
    "The interruption didn’t arrive. Interrupt again.",
  ),

  assistant_state_unknown: worded(
    "fixable",
    "The hub is newer than this version of Orbit. Update Orbit.",
    { label: "Check for updates", command: { command: "update_check" } },
  ),

  call_expired: { class: "silent" },
  call_ceiling_near: { class: "silent" },
  call_silence_resolved: { class: "silent" },
  call_silence_question: {
    class: "working",
    text: () => "Still there? If you don’t say anything, I’ll hang up in 15 seconds",
    actions: [
      { label: "Resume", command: { command: "call_keep" } },
      { label: "Hang up", command: { command: "call_stop" } },
    ],
  },
  call_preference_unreadable: worded(
    "fixable",
    "Couldn’t read your “When you start a video call” choice. Choose again in Settings.",
    openSettings,
  ),
  call_recording_started_automatically: worded("done", "Recording the call, as you chose."),
} satisfies Partial<Entries>;

export function conversationCopy(assistantName: string) {
  return {
    ...base,
    assistant_turn_lost_by_disconnect: worded(
      "fixable",
      `The connection dropped while ${assistantName} was responding. Ask again.`,
    ),
    assistant_outcome_unknown: worded(
      "fixable",
      `It’s unclear whether ${assistantName} responded. Ask again if needed.`,
    ),
    assistant_interrupt_refused: worded(
      "fixable",
      `Couldn’t interrupt ${assistantName}. Try again.`,
    ),
    approval_expired: worded(
      "fixable",
      `${assistantName}’s request had expired and wasn’t applied. Ask it to repeat the request.`,
    ),
    meeting_question_still_answering: worded(
      "fixable",
      `${assistantName} is still answering your previous question. Ask again when it finishes.`,
    ),
    bridge_serving: {
      class: "done",
      text: (data: NoticeData) => whatAssistantAsked(data, assistantName),
    },
  } as const;
}
