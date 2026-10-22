import { Fragment, useId } from "react";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { Spinner } from "../../design/Spinner";
import { es } from "../../i18n/es";
import { STEP_KEYS, isPreparing, noteFailure, stepLabel, waitsForHub } from "./note-progress";
import type { Send } from "./send";
import styles from "./Note.module.css";

/** «✓ Audio guardado — ◌ Transcribiendo… — ○ Escribir el resumen». */
function Steps({ note }: { note: OpenNoteSnapshot }) {
  return (
    <ol aria-label={es.noteScreen.progressLabel} className={styles.steps}>
      {STEP_KEYS.map((key, index) => {
        const step = note.progress[key];
        return (
          <Fragment key={key}>
            {index > 0 && <li aria-hidden="true" className={styles.stepLine} />}
            <li className={styles.step} data-state={step.state}>
              <span aria-hidden="true" className={styles.stepMark}>
                {step.state === "done" && <Icon name="check" size={14} />}
                {step.state === "active" && <Spinner size={16} />}
              </span>
              {stepLabel(key, step)}
            </li>
          </Fragment>
        );
      })}
    </ol>
  );
}

/** Not drawn in the design: why the note is missing, and «Reintentar». */
function Failure({ note, send }: { note: OpenNoteSnapshot; send: Send }) {
  const failure = noteFailure(note);
  if (!failure) return null;
  return (
    <section className={styles.progressCard} data-tone="failed" role="alert">
      <div className={styles.failureHead}>
        <span aria-hidden="true" className={styles.failureDot} />
        <h2>{failure.title}</h2>
      </div>
      <p>{failure.body}</p>
      <div className={styles.failureActions}>
        <Button onClick={() => void send(failure.retry)} size={32} variant="accent">
          {es.noteScreen.retry}
        </Button>
      </div>
    </section>
  );
}

/**
 * Above the note while Orbit prepares it: three steps, each with its state.
 * A note that failed says why instead, with «Reintentar».
 */
export function NoteProgress({ note, send }: { note: OpenNoteSnapshot; send: Send }) {
  const titleId = useId();
  if (noteFailure(note)) return <Failure note={note} send={send} />;
  if (!isPreparing(note)) return null;
  return (
    <section aria-labelledby={titleId} className={styles.progressCard}>
      <h2 id={titleId}>{es.noteScreen.preparing}</h2>
      <p>{es.noteScreen.preparingBody}</p>
      <Steps note={note} />
      {waitsForHub(note.progress) && (
        <p className={styles.stepNote}>{es.noteScreen.hubUnavailable}</p>
      )}
    </section>
  );
}
