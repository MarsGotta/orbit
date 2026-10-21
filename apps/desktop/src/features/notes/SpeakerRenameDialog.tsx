import { useState } from "react";
import type { SpeakerRenameScope } from "../../bridge/generated/SpeakerRenameScope";
import { Button } from "../../design/Button";
import { useModalFocus } from "../../design/use-modal-focus";
import { es } from "../../i18n/es";
import styles from "./SpeakerRenameDialog.module.css";

interface SpeakerRenameDialogProps {
  name: string;
  speakerName: string;
  atMs: number;
  adding: boolean;
  onChoose: (name: string) => void;
  onApply: (scope: SpeakerRenameScope) => void;
  onClose: () => void;
}

function ScopeChoices({
  all,
  onAll,
  speakerName,
}: {
  all: boolean;
  onAll: (all: boolean) => void;
  speakerName: string;
}) {
  return (
    <fieldset className={styles.choices}>
      <legend>{es.notes.speakerScope}</legend>
      <label>
        <input type="radio" name="speaker-scope" checked={!all} onChange={() => onAll(false)} />
        {es.notes.onlyThisTurn}
      </label>
      <label>
        <input type="radio" name="speaker-scope" checked={all} onChange={() => onAll(true)} />
        {es.notes.allSpeakerTurns(speakerName)}
      </label>
    </fieldset>
  );
}

export function SpeakerRenameDialog(props: SpeakerRenameDialogProps) {
  const [name, setName] = useState(props.name);
  const [all, setAll] = useState(false);
  const panel = useModalFocus(true);
  const title = props.adding ? es.notes.addSpeaker : es.notes.assignSpeaker(props.name);
  return (
    <div
      className={styles.backdrop}
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) props.onClose();
      }}
    >
      <section
        ref={panel}
        className={styles.dialog}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.stopPropagation();
            props.onClose();
          }
        }}
      >
        <h2>{title}</h2>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (props.adding) {
              if (name.trim()) props.onChoose(name.trim());
            } else props.onApply(all ? { kind: "voice" } : { kind: "turn", at_ms: props.atMs });
          }}
        >
          {props.adding ? (
            <label className={styles.name}>
              {es.notes.speakerName}
              <input
                aria-label={es.notes.speakerName}
                value={name}
                onChange={(event) => setName(event.currentTarget.value)}
              />
            </label>
          ) : (
            <ScopeChoices all={all} onAll={setAll} speakerName={props.speakerName} />
          )}
          <p>{es.notes.speakerNoteHelp}</p>
          <footer>
            <Button onClick={props.onClose}>{es.notes.cancel}</Button>
            <Button type="submit" variant="accent" disabled={props.adding && !name.trim()}>
              {props.adding ? es.notes.save : es.notes.applySpeaker}
            </Button>
          </footer>
        </form>
      </section>
    </div>
  );
}
