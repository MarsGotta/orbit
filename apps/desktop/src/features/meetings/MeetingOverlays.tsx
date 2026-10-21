import type { AppView as AppSnapshot } from "../../state/app-view";
import { useEffect, useRef, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import { TextLink } from "../../design/TextLink";
import { es } from "../../i18n/es";
import { MeetingStartDialog } from "./MeetingStartDialog";
import { Toast } from "../../design/Toast";
import type { VisibleClass } from "../../app/interceptor";

function SavedToast({ recordingId, onClose }: { recordingId: string; onClose: () => void }) {
  return (
    <Toast
      message={es.meeting.saved}
      action={<TextLink href={`#/notes/${recordingId}`}>{es.meeting.openNote}</TextLink>}
      onClose={onClose}
    />
  );
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
  return (
    <>
      {launcherOpen && snapshot.recording_availability?.state !== "unknown" && (
        <MeetingStartDialog onClose={onLauncherClose} send={send} snapshot={snapshot} />
      )}
      {savedId !== null && savedId !== "" && (
        <SavedToast key={savedId} recordingId={savedId} onClose={() => setSavedId(null)} />
      )}
      {error !== null && error !== "" && (
        <Toast message={error} kind={errorKind ?? "failed"} onClose={onErrorDismiss} />
      )}
    </>
  );
}
