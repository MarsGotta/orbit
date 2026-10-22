import type { AppCommand } from "../../bridge/generated";

/** The meeting the route names (`#/notes/<id>`), or "" when none. */
export function routeRecordingId(): string {
  const id = window.location.hash.match(/^#\/notes\/([^/?]+)/)?.[1] ?? "";
  try {
    return decodeURIComponent(id);
  } catch {
    return id;
  }
}

/** Opens a meeting: its route, and its note read from the hub. */
export function openMeeting(recordingId: string, send: (command: AppCommand) => unknown): void {
  window.location.hash = `#/notes/${encodeURIComponent(recordingId)}`;
  void send({ command: "notes_open", recording_id: recordingId });
}
