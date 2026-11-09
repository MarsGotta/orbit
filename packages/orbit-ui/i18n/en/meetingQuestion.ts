import { numberFormat } from "../locale";
// Deimos during the meeting — «Preguntar a Deimos», its answer in
// the floating panel, and «Oye Deimos». A question without answer
// says what happened, with the limits and recovery that apply.
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

    linkWithHost: (label: string, host: string) => `${label} (${host})`,
    linkRefused: "Couldn’t open the link.",
    causes: {
      privacyStopped: "Everything is stopped. Rearm Orbit to ask.",
      noMeeting: "No recording is in progress.",
      hubUnreachable: "No connection to the hub. Ask again when it reconnects.",
      tryAgain:
        "The answer is unavailable; Orbit received no cause. Check the conversation before asking again.",
      questionEmpty: "The question is empty.",
      questionTooLong: (bytes: number, max: number) =>
        `The question is ${numberFormat().format(bytes)} bytes; the limit is ${numberFormat().format(max)} bytes. Shorten it.`,
      windowNotOffered: (minutes: number, offered: number[]) =>
        `The requested ${numberFormat().format(minutes)} min window is unavailable. Choose from the offered minutes: ${offered.map((n) => numberFormat().format(n)).join(", ") || "none"}.`,
      turnFailed: "The assistant’s turn failed. Check the conversation before asking again.",
      noAnswer:
        "The assistant finished without an answer. Check the conversation or rephrase your question.",
      unavailable: (code: string) =>
        `The assistant is unavailable (${code}). Ask whoever manages your hub to check it.`,
      hubRefused: (code: string, message: string) =>
        `The hub refused the question (${code})${message ? `: ${message}` : ". No detail was provided."}`,
      sendFailed: (message: string) =>
        `The question could not be sent. Check the hub connection before trying again.${message ? ` Details: ${message}` : ""}`,
      refusalCodes: {
        upstream_unavailable:
          "The assistant is disconnected from the hub. Ask again when it reconnects.",
        upstream_rejected:
          "The assistant refused the request. Ask whoever manages your hub to check its configuration.",
        upstream_protocol:
          "The assistant sent an incompatible response. Ask whoever manages your hub to check the hub and assistant versions.",
        turn_outcome_unknown:
          "The hub cannot confirm what happened to the previous question. Check the conversation before asking again.",
      },
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
        localeUnsupported: "macOS doesn’t offer English speech recognition; use the button",
        unavailable: "this Mac doesn’t support it; use the button",
        failed: "speech recognition stopped; use the button",
        helperDisconnected: "speech recognition is unavailable; use the button",
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
