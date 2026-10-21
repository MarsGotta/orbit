import type { AppView as AppSnapshot } from "../../state/app-view";
import { useEffect, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import { Button } from "../../design/Button";
import { TextLink } from "../../design/TextLink";
import { es } from "../../i18n/es";
import styles from "./Meetings.module.css";

type AudioChoice = "microphone" | "system" | `application:${number}`;

function selectedAudio(
  choice: AudioChoice,
): import("../../bridge/generated/MeetingAudio").MeetingAudio {
  if (choice === "microphone") return "microphone_only";
  if (choice === "system") return choice;
  return { application: { application_pid: Number(choice.slice("application:".length)) } };
}

function blockedReason(snapshot: AppSnapshot, source: AudioChoice): string | null {
  if (snapshot.privacy_stopped) return es.meeting.privacyStopped;
  if (snapshot.permissions.microphone === "denied") return es.meeting.missingMicrophone;
  if (source !== "microphone" && snapshot.permissions.screen === "denied") {
    return es.meeting.missingScreen;
  }
  return null;
}

export interface MeetingStartDialogProps {
  onClose: () => void;
  send: (command: AppCommand) => void;
  snapshot: AppSnapshot;
}

/**
 * Why no application can be chosen, only when the Mac said none plays audio:
 * a list not read yet or not readable says nothing; it stops nothing.
 */
function emptyReason(snapshot: AppSnapshot): string | undefined {
  return snapshot.meeting.applications_state === "empty"
    ? es.meeting.sourceNoApplications
    : undefined;
}

function ApplicationOptions({ snapshot }: { snapshot: AppSnapshot }) {
  if (snapshot.meeting.applications.length === 0) {
    return (
      <option disabled title={emptyReason(snapshot)} value="application">
        {es.meeting.sourceApplication}
      </option>
    );
  }
  return snapshot.meeting.applications.map((application) => (
    <option key={application.pid} value={`application:${application.pid}`}>
      {application.name}
    </option>
  ));
}

function SourceField({
  onChange,
  snapshot,
  source,
}: {
  onChange: (choice: AudioChoice) => void;
  snapshot: AppSnapshot;
  source: AudioChoice;
}) {
  return (
    <label className={styles.field}>
      {es.meeting.sourceLabel}
      <select
        aria-label={es.meeting.sourceLabel}
        onChange={(event) => onChange(event.currentTarget.value as AudioChoice)}
        value={source}
      >
        <option value="system">{es.meeting.sourceSystem}</option>
        <option value="microphone">{es.meeting.sourceMicrophone}</option>
        <ApplicationOptions snapshot={snapshot} />
      </select>
    </label>
  );
}

export function MeetingStartDialog({ onClose, send, snapshot }: MeetingStartDialogProps) {
  const [title, setTitle] = useState("");
  const [source, setSource] = useState<AudioChoice>("system");
  // The daemon only reads the inventory when asked, so an unasked dialog can
  // never offer a single application.
  useEffect(() => {
    send({ command: "audio_sources_refresh" });
  }, [send]);
  const blocked = blockedReason(snapshot, source);
  const start = () => {
    send({
      command: "meeting_start",
      title: title.trim(),
      minutes: 60,
      audio: selectedAudio(source),
    });
    onClose();
  };
  return (
    <div className={styles.backdrop} role="presentation">
      <section aria-label={es.meeting.startDialog} className={styles.dialog} role="dialog">
        <header className={styles.dialogHeader}>
          <h2>{es.meeting.startDialog}</h2>
          <Button aria-label={es.meeting.close} onClick={onClose} size={28} variant="ghost">
            ×
          </Button>
        </header>
        <label className={styles.field}>
          {es.meeting.titleLabel}
          <input
            placeholder={es.meeting.defaultTitle}
            onChange={(event) => setTitle(event.currentTarget.value)}
            value={title}
          />
        </label>
        <SourceField onChange={setSource} snapshot={snapshot} source={source} />
        {Boolean(blocked) && (
          <p className={styles.blocked}>
            {blocked} · <TextLink href="#/settings">{es.meeting.openSettings}</TextLink>
          </p>
        )}
        <footer className={styles.dialogActions}>
          <Button onClick={onClose}>{es.meeting.close}</Button>
          <Button disabled={blocked !== null} onClick={start} variant="accent">
            {blocked ?? es.meeting.record}
          </Button>
        </footer>
      </section>
    </div>
  );
}
