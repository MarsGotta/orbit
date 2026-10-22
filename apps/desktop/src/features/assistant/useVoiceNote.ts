import { useEffect, useRef, useState, type PointerEvent } from "react";
import type { AppCommand } from "../../bridge/generated";

/** The order to the daemon; what it returns settles once the daemon has answered. */
export type VoiceSend = (command: AppCommand) => unknown;

/**
 * How long the microphone button must be held for its release to send the
 * note. A shorter press is a click, which starts a note that the next click
 * sends. A deliberate click lasts around a tenth of a second; talking while
 * holding the button lasts at least a word, longer than this. It bounds
 * nothing else: not the note's length nor the time between the two clicks.
 */
export const HOLD_TO_TALK_MS = 300;

/** How often the elapsed time of a note in progress is repainted. */
const ELAPSED_REPAINT_MS = 250;

/**
 * The voice note the composer started: `holding` until the button is
 * released, `clicked` when the press was a click (or the system took the
 * pointer away), so that the next click or «Enviar nota de voz» sends it.
 */
type Press = { mode: "idle" } | { mode: "holding"; since: number } | { mode: "clicked" };

/** Milliseconds since `recording` became true; zero while it is false. */
function useElapsed(recording: boolean): number {
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [now, setNow] = useState(0);
  useEffect(() => {
    if (!recording) {
      setStartedAt(null);
      return;
    }
    const started = Date.now();
    setStartedAt(started);
    setNow(started);
    const timer = window.setInterval(() => setNow(Date.now()), ELAPSED_REPAINT_MS);
    return () => window.clearInterval(timer);
  }, [recording]);
  return startedAt === null ? 0 : Math.max(0, now - startedAt);
}

/** Esc cancels the note while it records, wherever the focus is. */
function useEscapeCancels(recording: boolean, cancel: () => void) {
  const latest = useRef(cancel);
  latest.current = cancel;
  useEffect(() => {
    if (!recording) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      latest.current();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [recording]);
}

/**
 * What the daemon says about the microphone, as a ref for the settle of an
 * order, and the end of the composer's press when the daemon ends the note
 * itself (a stop, the shortcut, a cut).
 */
function useDaemonListening(daemonListening: boolean, endPress: () => void) {
  const listening = useRef(daemonListening);
  const was = useRef(daemonListening);
  useEffect(() => {
    listening.current = daemonListening;
    if (was.current && !daemonListening) endPress();
    was.current = daemonListening;
  }, [daemonListening, endPress]);
  return listening;
}

interface VoiceNoteInput {
  daemonListening: boolean;
  /** Whether a new note may start now; a note in progress can always end. */
  canStart: boolean;
  send: VoiceSend;
}

export function useVoiceNote({ canStart, daemonListening, send }: VoiceNoteInput) {
  const [press, setPress] = useState<Press>({ mode: "idle" });
  const endPress = useRef(() => setPress({ mode: "idle" })).current;
  const listening = useDaemonListening(daemonListening, endPress);
  // The release after the press that sent the note does nothing more.
  const swallowRelease = useRef(false);
  const recording = press.mode !== "idle" || daemonListening;

  const start = (mode: Press) => {
    if (!canStart) return;
    setPress(mode);
    // Once the daemon has answered, the composer believes it: a refused
    // start must not go on saying «Grabando».
    void Promise.resolve(send({ command: "voice_start" })).then(() => {
      if (!listening.current) endPress();
    });
  };
  const finish = () => {
    endPress();
    send({ command: "voice_end" });
  };
  const cancel = () => {
    endPress();
    send({ command: "voice_cancel" });
  };
  useEscapeCancels(recording, cancel);

  const pointerDown = (event: PointerEvent<HTMLButtonElement>) => {
    if (event.button !== 0 || press.mode === "holding") return;
    if (recording) {
      swallowRelease.current = true;
      finish();
      return;
    }
    // Releasing anywhere, even outside the button, reaches the button.
    event.currentTarget.setPointerCapture(event.pointerId);
    start({ mode: "holding", since: Date.now() });
  };
  const pointerUp = () => {
    if (swallowRelease.current) swallowRelease.current = false;
    else if (press.mode === "holding")
      if (Date.now() - press.since >= HOLD_TO_TALK_MS) finish();
      else setPress({ mode: "clicked" });
  };
  return {
    elapsedMs: useElapsed(recording),
    finish,
    recording,
    pointerDown,
    pointerUp,
    // The system took the pointer: the note goes on, and says so, until a
    // click sends it or Esc cancels it.
    pointerCancel: () => press.mode === "holding" && setPress({ mode: "clicked" }),
    // Enter or Space on the focused button arrives with no pointer: a click.
    keyboardClick: (event: { detail: number }) => {
      if (event.detail !== 0) return;
      if (recording) finish();
      else start({ mode: "clicked" });
    },
  };
}
