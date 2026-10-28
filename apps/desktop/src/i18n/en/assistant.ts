import { pluralRules } from "../locale";
import { numberFormat } from "../locale";
import { assistantBase } from "./assistantBase";
import { voiceFailureDetail, voiceFailureText } from "./voice-failures";

const secondsFormat = numberFormat({ maximumFractionDigits: 2 });

/** Milliseconds as seconds, the Spanish way: 90 is «0,09». */
function seconds(ms: number): string {
  return secondsFormat.format(ms / 1_000);
}

const base = {
  chat: {
    inputLabel: "Message",
    send: "Send",
    interrupt: "Interrupt",
    clarifyLabel: "Answer",
    clarify: "Reply",
    // The hub's approval choices (hub-v1 «Choices and questions»).
    choices: {
      once: "Allow once",
      session: "In this conversation",
      always: "Always",
      deny: "Don’t allow",
    },
    questionOf: (position: number, total: number) =>
      `Question ${numberFormat().format(position)} of ${numberFormat().format(total)}`,
  },
  assistant: {
    ...assistantBase,
    transcribingVoice: "Transcribing your voice…",

    transcribing: "Transcribing…",
    voiceFailure: voiceFailureText,
    voiceFailureDetail,
    audioUnavailable: "Audio unavailable",
    voiceProgress: "Voice note progress",
    voiceNoteRecording: (elapsed: string) => `Recording voice note, ${elapsed}. Press to send`,
    sendVoiceNote: "Send voice note",
    voiceNoteTooShort: (durationMs: number, minimumMs: number) =>
      `The voice note was ${seconds(durationMs)} s long; the minimum is ${seconds(minimumMs)} s. It wasn’t sent.`,
    retryTranscription: "Retry transcription",
    discardVoiceNote: "Discard",
    discardVoiceNoteTitle: "Discard this voice note?",
    discardVoiceNoteText:
      "It will leave the conversation and its audio will be deleted from this Mac.",

    loadMoreSessions: "Load more conversations",
    loadingSessions: "Loading conversations…",
    sessionsFailed: "Couldn’t load conversations.",
    retrySessions: "Try again",
    // What the agent did in a turn (hub-v1 chat.item).
    work: {
      label: "Activity",
      working: (duration: string) => `Working · ${duration}`,
      worked: (duration: string) => `Worked for ${duration}`,
      used: (subjects: string) => `Used ${subjects}`,
      steps: (count: number) =>
        pluralRules().select(count) === "one" ? "1 step" : `${numberFormat().format(count)} steps`,
      reasoning: "Reasoning",
      stepStatus: {
        pending: "Pending",
        in_progress: "In progress",
        completed: "Done",
        cancelled: "Canceled",
      },
    },
    loadOlder: "Load earlier messages",
    loadingOlder: "Loading earlier messages…",
    historyFailed: "Couldn’t load messages.",
  },
} as const;

/** «1 minuto», «1 min 30 s», «2 minutos»: the silence that hangs a call up. */

/** What the composer says while it cannot send: why, and that the text stays. */
function keptWhile(reason: string): string {
  return `${reason} Your text stays here until you can send it.`;
}

/** Why the composer cannot send now; what is written stays. */
function blockedCopy(assistantName: string) {
  return {
    permission: keptWhile(`${assistantName} is waiting for your permission.`),
    question: keptWhile(`${assistantName} is waiting for your answer.`),
    answering: keptWhile(`${assistantName} is responding.`),
    transcribing: keptWhile("Your voice note is being transcribed."),
    uploading: "An attachment is uploading. You can send as soon as it finishes.",
    offline: keptWhile("No connection to your hub."),
    starting: keptWhile("Orbit is still connecting."),
    sending: "Sending your message…",
  };
}

/** What the call says, in the agent's name. */
function callCopy(assistantName: string) {
  return {
    title: `Call with ${assistantName}`,
    speaking: `${assistantName} is speaking`,
    listening: "Listening to you",
    connecting: `Calling ${assistantName}…`,
    you: (question: string) => `You: ${question}`,
    seesScreen: "Can see your screen",
    interrupt: "Speak at any time to interrupt",
    seeConversation: "View conversation",
    minimised: "Call minimized",
    limits:
      "After 30 s of silence, asks whether to continue; hangs up after 15 s without an answer · ⌃⌥D hangs up from any app",
  } as const;
}

export function assistantCopy(assistantName: string) {
  return {
    ...base,
    chat: { ...base.chat, roles: { assistant: assistantName } },
    assistant: {
      ...base.assistant,
      thinking: `${assistantName} is thinking…`,
      awaitingPermission: `${assistantName} is waiting for your permission`,
      awaitingAnswer: `${assistantName} is waiting for your answer`,
      retryingOpenai: "OpenAI is slow to respond; retrying",
      retryingAnthropic: "Anthropic is slow to respond; retrying",
      responding: `${assistantName} is responding…`,
      noHermesVoice: `${assistantName} replies in text`,
      // The list of conversations.
      conversations: "Conversations",
      newShort: "New",
      now: "Now",
      messageCount: (count: number) =>
        count === 0
          ? "No messages"
          : pluralRules().select(count) === "one"
            ? "1 message"
            : `${numberFormat().format(count)} messages`,
      // The header of the open conversation.
      waitsPermission: "waiting for your permission",
      asksSomething: "asking you a question",
      working: "working…",
      newConversationState: "new conversation",
      // The thread.
      emptyTitle: `Ask ${assistantName}`,
      emptyText: "Write below or use the microphone to record a voice note.",
      thread: "Conversation",
      loadingAssistant: "Reading your conversations from the hub…",
      answerLabel: `Answer from ${assistantName}`,
      workingLabel: `${assistantName} working`,
      yourMessage: "Your message",
      turnFailed: `${assistantName} couldn’t respond. Ask again.`,
      askAgain: "Ask again",
      // The composer.
      composerLabel: `Message to ${assistantName}`,
      composerPlaceholder: `Write to ${assistantName}…`,
      // What was written before Orbit closed or fell, back in the box.
      draftRecovered: "Recovered your unsent draft.",
      blocked: blockedCopy(assistantName),
      recordingNote: "Recording voice note",
      escCancels: "· esc cancels",
      cancel: "Cancel",
      voiceNoteButton: "Record voice note",
      // What the agent waits for.
      approvalTitle: `${assistantName} needs your permission`,
      clarifyTitle: `${assistantName} asks`,
      decided: {
        once: "You allowed it once.",
        session: "You allowed it in this conversation.",
        always: "You allowed it always.",
        deny: "You didn’t allow it.",
      } as const,
      stepWaits: "waiting for your permission",
      stepDenied: "you didn’t allow it",
      // A Mac without its hub.
      unpaired: `Pair this Mac with your hub to talk to ${assistantName}.`,
      pairMac: "Pair this Mac",
      // «Editar y copiar».
      editorSubtitle: `Edit and copy it, or send it to ${assistantName} as a new message.`,
    },
    call: callCopy(assistantName),
  } as const;
}
