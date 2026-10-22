import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Checkbox } from "../../design/Checkbox";
import { es } from "../../i18n/es";
import { actionsMarkdown, groupActions } from "./action-groups";
import { CopyButton } from "./copy-button";
import { sent, type Send } from "./send";
import styles from "./Note.module.css";

/**
 * The tasks of the note, by person, the user's first. A mark shows at once
 * with «Guardando…» beside it until the hub has it; it never turns back by itself.
 */
export function ActionsTab({ note, send }: { note: OpenNoteSnapshot; send: Send }) {
  if (note.actions.length === 0) {
    return (
      <p className={styles.quiet}>
        {note.summary === null ? es.noteScreen.actionsLater : es.noteScreen.noActions}
      </p>
    );
  }
  const groups = groupActions(note.actions, note.owner_name);
  const left = note.actions.filter((action) => !action.done).length;
  return (
    <div className={styles.actions}>
      <div className={styles.toolbar}>
        <span className={styles.toolbarMeta}>{es.noteScreen.actionsLeft(left)}</span>
        <CopyButton label={es.noteScreen.copyActions} text={() => actionsMarkdown(groups)} />
      </div>
      {groups.map((group) => (
        <section className={styles.actionGroup} key={group.key}>
          <h3>{group.label}</h3>
          {group.items.map((action) => (
            <div className={styles.actionRow} data-done={action.done} key={action.id}>
              <Checkbox
                checked={action.done}
                onCheckedChange={(done) =>
                  sent(send, {
                    command: "note_action_toggle",
                    recording_id: note.recording_id,
                    action_id: action.id,
                    done,
                  })
                }
              >
                <span className={styles.actionText}>{action.text}</span>
                {action.due != null && action.due !== "" && (
                  <span className={styles.actionDue}>{es.noteScreen.actionDue(action.due)}</span>
                )}
              </Checkbox>
            </div>
          ))}
        </section>
      ))}
    </div>
  );
}
