import { useEffect, useRef, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { AppView as AppSnapshot } from "../../state/app-view";
import type { ApplicationsKnown, AudioSourceFieldProps } from "./AudioSourceField";
import { selectedAudio, type AudioSource } from "./start-options";

// How long the dialog says it is looking for applications before it says it
// got no list. The inventory is one local question to the Mac's helper; a
// list still missing after ten seconds is not coming (the daemon can leave it
// unknown until it is asked again), and «Volver a buscar» asks again. It
// bounds only the words on screen, never the request.
const SEARCH_MS = 10_000;

/**
 * The applications that play audio, asked for when the dialog opens: the
 * daemon reads the inventory only when asked. Until it answers the list is
 * `unavailable`, a state that also covers a list that could not be read: it
 * is said as «Buscando…» for `SEARCH_MS`, and as not known after.
 */
function useApplicationsKnown(snapshot: AppSnapshot, send: (command: AppCommand) => void) {
  const [asked, setAsked] = useState(0);
  const [gaveUp, setGaveUp] = useState(false);
  useEffect(() => {
    send({ command: "audio_sources_refresh" });
    setGaveUp(false);
    const timer = window.setTimeout(() => setGaveUp(true), SEARCH_MS);
    return () => window.clearTimeout(timer);
  }, [send, asked]);
  const state = snapshot.meeting.applications_state;
  let known: ApplicationsKnown = "unknown";
  if (state === "ready" || state === "empty") known = state;
  else if (state === "unavailable" && !gaveUp) known = "searching";
  return { known, searchAgain: () => setAsked((count) => count + 1) };
}

/**
 * «Empezar a grabar» from the press until the recording begins (a new
 * recording in the meeting) or a new refusal arrives, which the dialog says.
 * A press clears the refusal on screen, so the same one said again is new.
 */
export function useStart(
  snapshot: AppSnapshot,
  refusals: { error: string | null; onErrorDismiss: () => void },
  onClose: () => void,
) {
  const { error, onErrorDismiss } = refusals;
  const [pending, setPending] = useState(false);
  const [refusal, setRefusal] = useState<string | null>(null);
  const before = useRef<{ recording: string | null; error: string | null }>({
    recording: null,
    error: null,
  });
  const recording = snapshot.meeting.recording_id;
  const begun = snapshot.meeting.phase !== "idle" && recording !== before.current.recording;
  useEffect(() => {
    if (pending && begun) onClose();
  }, [pending, begun, onClose]);
  useEffect(() => {
    // Once cleared, the same words are a new refusal.
    if (error === null) before.current.error = null;
    if (!pending || error === null || error === before.current.error) return;
    setPending(false);
    setRefusal(error);
  }, [pending, error]);
  const press = () => {
    if (error !== null) onErrorDismiss();
    before.current = { recording, error };
    setRefusal(null);
    setPending(true);
  };
  return { pending, refusal, press };
}

/**
 * What audio to record: the source and, for an application, the chip chosen
 * (the first one that plays until another is pressed). `audio` is null while
 * no application can be named.
 */
export function useAudioChoice(snapshot: AppSnapshot, send: (command: AppCommand) => void) {
  const [source, setSource] = useState<AudioSource>("system");
  const [pid, setPid] = useState<number | null>(null);
  const { known, searchAgain } = useApplicationsKnown(snapshot, send);
  const applications = snapshot.meeting.applications;
  const chosen = applications.find((app) => app.pid === pid) ?? applications[0] ?? null;
  const field: Omit<AudioSourceFieldProps, "statusId"> = {
    source,
    onSource: setSource,
    applications,
    known,
    chosen,
    onChoose: setPid,
    onSearchAgain: searchAgain,
  };
  const audio = selectedAudio(source, known === "ready" && chosen ? chosen.pid : null);
  return { audio, field };
}
