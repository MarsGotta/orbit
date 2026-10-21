import type { AppView as AppSnapshot } from "../../state/app-view";

import type { ViewId } from "../../state/ui";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";

export function MeetingEntryPoints({
  onRecord,
  snapshot,
  view,
}: {
  onRecord: () => void;
  snapshot: AppSnapshot;
  view: ViewId;
}) {
  // Calendar owns its own Meet button in the view toolbar (APP-03/APP-04);
  // while a meeting is active the shell status pill takes over.
  if (view === "calendar" || snapshot.meeting.phase !== "idle") return null;
  // SIM-24: while the recording is not known there is nothing to offer or say.
  if (snapshot.recording_availability?.state === "unknown") return null;
  return (
    <Button onClick={onRecord} size={28} variant="accent">
      {es.meeting.meet}
    </Button>
  );
}
