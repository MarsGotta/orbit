import { useCallback } from "react";
import type { NoteSpeakerSnapshot } from "../../bridge/generated/NoteSpeakerSnapshot";
import type { NoteTurnSnapshot } from "../../bridge/generated/NoteTurnSnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import type { SpeakerRenameScope } from "../../bridge/generated/SpeakerRenameScope";
import { Popover } from "../../design/Popover";
import { es } from "../../i18n/es";
import { SpeakerMenu } from "./SpeakerMenu";
import { SpeakerName } from "./TranscriptSegment";
import styles from "./Note.module.css";

/** Who speaks in a turn: a button that opens «¿Quién es…?» anchored to it. */
export function SpeakerControl({
  note,
  open,
  setOpen,
  speaker,
  turn,
  onApply,
}: {
  note: OpenNoteSnapshot;
  open: boolean;
  setOpen: (turn: number | null) => void;
  speaker: NoteSpeakerSnapshot | undefined;
  turn: NoteTurnSnapshot;
  onApply: (name: string, scope: SpeakerRenameScope, turns: number) => Promise<unknown>;
}) {
  const index = turn.index;
  const change = useCallback((next: boolean) => setOpen(next ? index : null), [index, setOpen]);
  return (
    <Popover
      label={es.notes.whoIs(speaker?.name ?? turn.speaker_key)}
      onOpenChange={change}
      open={open}
      trigger={<SpeakerName speaker={speaker} turn={turn} />}
      triggerClassName={styles.speakerButton}
    >
      <SpeakerMenu note={note} onApply={onApply} speaker={speaker} turn={turn} />
    </Popover>
  );
}
