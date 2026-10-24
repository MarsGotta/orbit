import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AppCommand } from "../../bridge/generated";
import { MeetingStartDialog } from "./MeetingStartDialog";
import { Toast } from "../../design/Toast";
import type { VisibleClass } from "../../app/interceptor";

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
  // «Reunión guardada» and its note are MeetingNoteNotice's, in the corner.
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
      {/* While the dialog is open it says the refusal itself, over the corner. */}
      {!dialogOpen && error !== null && error !== "" && (
        <Toast message={error} kind={errorKind ?? "failed"} onClose={onErrorDismiss} />
      )}
    </>
  );
}
