import { Fragment } from "react";
import type { NoteProgressSnapshot } from "../../bridge/generated/NoteProgressSnapshot";
import { es } from "../../i18n/es";
import { STEP_KEYS, causeText, progressVisible } from "./note-progress";
import styles from "./NoteScreen.module.css";

/**
 * Upload → Transcript → Summary, centred above the note while anything is
 * left (P9-04). Each step says its state; where it stands is its tooltip
 * and its accessible description.
 */
export function NoteProgress({ progress }: { progress: NoteProgressSnapshot }) {
  if (!progressVisible(progress)) return null;
  return (
    <ol aria-label={es.noteScreen.progressLabel} className={styles.pipeline}>
      {STEP_KEYS.map((key, index) => {
        const step = progress[key];
        const cause = (step.cause && causeText(step.cause)) ?? es.noteScreen.stepStates[step.state];
        return (
          <Fragment key={key}>
            {index > 0 && <li aria-hidden="true" className={styles.pipelineArrow} />}
            <li className={styles.pipelineStep} data-state={step.state} title={cause}>
              <span aria-hidden="true" className={styles.pipelineDot} />
              <span>{es.noteScreen.steps[key]}</span>
              <span className={styles.visuallyHidden}>
                {`: ${es.noteScreen.stepStates[step.state]}. ${cause}`}
              </span>
            </li>
          </Fragment>
        );
      })}
    </ol>
  );
}
