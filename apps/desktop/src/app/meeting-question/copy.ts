import { getCopy } from "../../i18n";
import type { MeetingQuestionCause } from "../../bridge/generated/MeetingQuestionCause";
import type { VoiceOffCause } from "../../bridge/generated/VoiceOffCause";
import type { VoiceStatusSnapshot } from "../../bridge/generated/VoiceStatusSnapshot";

/** Each cause of a question without answer, as what to do. */
export function causeText(cause: MeetingQuestionCause, copy = getCopy()): string {
  const causes = copy.meetingQuestion.causes;
  switch (cause.cause) {
    case "privacy_stopped":
      return causes.privacyStopped;
    case "no_meeting":
      return causes.noMeeting;
    case "hub_unreachable":
      return causes.hubUnreachable;
    case "turn_in_flight":
      return copy.meetingQuestion.busy[cause.turn];
    case "question_empty":
      return causes.questionEmpty;
    case "question_too_long":
      return causes.questionTooLong;
    case "window_not_offered":
      return causes.windowNotOffered;
    case "interrupted":
      return causes.interrupted;
    case "revoked":
      return causes.revoked;
    case "deimos_unavailable":
    case "turn_failed":
    case "hub_refused":
    case "no_answer":
      return causes.tryAgain;
  }
}

/**
 * Whether «Oye Deimos» listens during the recording and, when not,
 * why, as the user can act on it; `null` outside a recording or while it gets
 * ready (nothing to say).
 */
export function voiceStatusText(
  status: VoiceStatusSnapshot | undefined,
  copy = getCopy(),
): string | null {
  if (!status) return null;
  switch (status.state) {
    case "listening":
      return copy.meetingQuestion.voice.listening;
    case "starting":
      return null;
    case "downloading":
      return copy.meetingQuestion.voice.downloading(status.percent ?? 0);
    case "not_listening":
      return copy.meetingQuestion.voice.off(offText(status.why));
    case "abandoned":
      return copy.meetingQuestion.voice.abandoned[status.cause];
  }
}

/** The short form for the pill; the full sentence goes in its title. */
export function voiceStatusShort(status: VoiceStatusSnapshot, copy = getCopy()): string | null {
  switch (status.state) {
    case "listening":
    case "abandoned":
      return copy.meetingQuestion.voice.short.listening;
    case "starting":
      return null;
    case "downloading":
      return copy.meetingQuestion.voice.short.downloading(status.percent ?? 0);
    case "not_listening":
      return copy.meetingQuestion.voice.short.off;
  }
}

function offText(why: VoiceOffCause): string {
  const reasons = getCopy().meetingQuestion.voice.why;
  switch (why.cause) {
    case "privacy_stopped":
      return reasons.privacyStopped;
    case "paused":
      return reasons.paused;
    case "macos_version":
      return reasons.macosVersion;
    case "locale_unsupported":
      return reasons.localeUnsupported;
    case "unavailable":
      return reasons.unavailable;
    case "failed":
      return reasons.failed;
    case "helper_disconnected":
      return reasons.helperDisconnected;
    case "helper_without_phrase":
      return reasons.helperWithoutPhrase;
    case "no_microphone":
      return reasons.noMicrophone;
  }
}

/** A web link in an answer: Markdown `[texto](url)` or a bare http(s) URL. */
export type AnswerPiece = { text: string } | { text: string; url: string };

const LINK = /\[([^\]\n]+)\]\((https?:\/\/[^\s)]+)\)|https?:\/\/[^\s<>"«»]+/g;

/** The answer split into text and web links; anything else stays text. */
export function answerPieces(answer: string): AnswerPiece[] {
  const pieces: AnswerPiece[] = [];
  let last = 0;
  for (const match of answer.matchAll(LINK)) {
    let whole = match[0];
    let url = match[2] ?? whole;
    let label = match[1] ?? url;
    if (match[2] === undefined) {
      // A bare URL does not take the sentence's closing punctuation.
      const trimmed = whole.replace(/[.,;:!?)]+$/, "");
      whole = trimmed;
      url = trimmed;
      label = trimmed;
    }
    const start = match.index;
    if (start > last) pieces.push({ text: answer.slice(last, start) });
    pieces.push({ text: label, url });
    last = start + whole.length;
  }
  if (last < answer.length) pieces.push({ text: answer.slice(last) });
  return pieces;
}

/** Where a link leads: its host, or the address itself if it has none. */
export function linkHost(url: string): string {
  try {
    return new URL(url).hostname || url;
  } catch {
    return url;
  }
}

/** A link's text: a Markdown label carries its host beside it, so the label
 *  can never hide where the link leads. */
export function linkText(piece: { text: string; url: string }): string {
  return piece.text === piece.url
    ? piece.text
    : getCopy().meetingQuestion.linkWithHost(piece.text, linkHost(piece.url));
}

/** UTF-8 bytes of the question, the unit the daemon's ceiling counts. */
export function questionBytes(question: string): number {
  return new TextEncoder().encode(question.trim()).length;
}
