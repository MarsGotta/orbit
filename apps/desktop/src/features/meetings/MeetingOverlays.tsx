import type { AppView as AppSnapshot } from "../../state/app-view";
import { useEffect, useRef, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import { es } from "../../i18n/es";
import { MeetingStartDialog } from "./MeetingStartDialog";
import { Toast } from "../../design/Toast";
import type { VisibleClass } from "../../app/interceptor";

/**
 * The recording is saved; its note takes about half a minute more, and
 * «Tu nota está lista» opens it once it is, so this one opens nothing.
 */
function SavedToast({ onClose }: { onClose: () => void }) {
  return <Toast detail={es.meeting.savedDetail} message={es.meeting.saved} onClose={onClose} />;
}

export interface MeetingOverlaysProps {
  error: string | null;
  errorKind?: VisibleClass;
  onErrorDismiss: () => void;
  launcherOpen: boolean;
  onLauncherClose: () => void;
  send: (command: AppCommand) => void;
  snapshot: AppSnapshot;
}

export function MeetingOverlays({
  error,
  errorKind,
  onErrorDismiss,
  launcherOpen,
  onLauncherClose,
  send,
  snapshot,
}: MeetingOverlaysProps) {
  const previous = useRef(snapshot.meeting);
  const [savedId, setSavedId] = useState<string | null>(null);
  useEffect(() => {
    const prior = previous.current;
    if (prior.phase !== "idle" && snapshot.meeting.phase === "idle") {
      const recent = snapshot.meeting.recent.find(
        (meeting) => meeting.recording_id === prior.recording_id,
      );
      if (recent) setSavedId(recent.recording_id);
    }
    previous.current = snapshot.meeting;
  }, [snapshot.meeting]);
  const dialogOpen = launcherOpen && snapshot.recording_availability?.state !== "unknown";
  return (
    <>
      {dialogOpen && (
        <MeetingStartDialog
          error={error}
          onClose={onLauncherClose}
          onErrorDismiss={onErrorDismiss}
          send={send}
          snapshot={snapshot}
        />
      )}
      {savedId !== null && savedId !== "" && (
        <SavedToast key={savedId} onClose={() => setSavedId(null)} />
      )}
      {/* While the dialog is open it says the refusal itself, over the corner. */}
      {!dialogOpen && error !== null && error !== "" && (
        <Toast message={error} kind={errorKind ?? "failed"} onClose={onErrorDismiss} />
      )}
    </>
  );
}
