import { numberFormat } from "../locale";
// Deimos during the meeting — «Preguntar a Deimos», its answer in
// the floating panel, and «Oye Deimos». A question without answer
// says what to do; its codes and sizes stay in the log.
const base = {
  meetingQuestion: {
    askAgain: "Another question",

    placeholder: "What did Riley say about the budget?",
    windowLabel: "Meeting context",
    window: (minutes: number) => `Last ${numberFormat().format(minutes)} min`,
    send: "Ask",
    cancel: "Cancel",
    close: "Close",

    asked: (question: string) => `Your question: ${question}`,

    tooLong: "The question is too long. Shorten it.",

    linkWithHost: (label: string, host: string) => `${label} (${host})`,
    linkRefused: "Couldn’t open the link.",
    causes: {
      privacyStopped: "Everything is stopped. Rearm Orbit to ask.",
      noMeeting: "No recording is in progress.",
      hubUnreachable: "No connection to the hub. Ask again when it reconnects.",
      tryAgain: "Ask again in a moment.",
      questionEmpty: "The question is empty.",
      questionTooLong: "The question is too long. Shorten it.",
      windowNotOffered: "Choose another time window.",
      interrupted: "You interrupted the answer.",
      revoked: "The hub revoked this Mac. Pair it again in Settings.",
    },
    busy: {
      voice_note: "You’re recording a voice note.",
      voice_transcription: "Your voice note is still being transcribed.",
    },
    voice: {
      why: {
        privacyStopped: "everything is stopped",
        paused: "recording is paused",
        macosVersion: "requires macOS 26; use the button",
        localeUnsupported: "macOS doesn’t offer Spanish speech recognition; use the button",
        unavailable: "this Mac doesn’t support it; use the button",
        failed: "speech recognition stopped; use the button",
        helperDisconnected: "Orbit.app isn’t running",
        helperWithoutPhrase: "update Orbit",
        noMicrophone: "the recording has no microphone yet",
      },
    },
  },
} as const;

export function meetingQuestionCopy(assistantName: string) {
  return {
    ...base,
    meetingQuestion: {
      ...base.meetingQuestion,
      ask: `Ask ${assistantName}`,
      fieldLabel: `Question for ${assistantName}`,
      panelLabel: `${assistantName} in the meeting`,
      thinking: `${assistantName} is thinking…`,
      responding: `${assistantName} is responding…`,
      alsoInConversation: `Also saved in your conversation with ${assistantName}`,
      failed: `${assistantName} didn’t answer`,
      notSent: `The question didn’t reach ${assistantName}`,
      busy: {
        ...base.meetingQuestion.busy,
        assistant_turn: `${assistantName} is still answering the previous question. Wait or interrupt.`,
        pending_approval: `${assistantName} is waiting for your answer in the conversation.`,
        call: `A call with ${assistantName} is open.`,
      },
      voice: {
        ...base.meetingQuestion.voice,
        listening: `“Hey ${assistantName}” is listening`,
        downloading: (percent: number) =>
          `“Hey ${assistantName}” isn’t listening yet: downloading speech recognition (${numberFormat().format(percent)}%)`,
        off: (why: string) => `“Hey ${assistantName}” isn’t listening: ${why}`,
        // «Oye …» was heard and no question followed: why, and what to do.
        abandoned: {
          phrase_revised: `I didn’t catch “Hey ${assistantName}”. Repeat it followed by your question.`,
          question_wait_expired: `I heard “Hey ${assistantName}”, but no question followed. Say it right after the name.`,
          question_too_long: `I heard “Hey ${assistantName}”, but couldn’t make out a question. Try “Ask ${assistantName}”.`,
        },
        short: {
          listening: `“Hey ${assistantName}” is listening`,
          downloading: (percent: number) =>
            `“Hey ${assistantName}” ${numberFormat().format(percent)}%`,
          off: `“Hey ${assistantName}” isn’t listening`,
        },
      },
    },
  } as const;
}
