import type { AppCommand } from "../../bridge/generated";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import { actionsMarkdown, groupActions } from "./action-groups";
import styles from "./NoteScreen.module.css";

/** The tasks of the note, grouped by who owns them. */
export function ActionsTab({
  note,
  send,
}: {
  note: OpenNoteSnapshot;
  send: (command: AppCommand) => void;
}) {
  if (note.actions.length === 0) {
    return (
      <section className={styles.emptyTab}>
        <p>{note.summary === null ? es.notes.actionsLater : es.notes.noActions}</p>
      </section>
    );
  }
  const groups = groupActions(note.actions, note.owner_name);
  return (
    <section className={styles.actions}>
      <div className={styles.toolbar}>
        <Button
          onClick={() => void navigator.clipboard?.writeText(actionsMarkdown(groups))}
          size={26}
          variant="ghost"
        >
          {es.noteScreen.copyActions}
        </Button>
      </div>
      {groups.map((group) => (
        <section className={styles.actionGroup} data-mine={group.mine} key={group.key}>
          <h2>{group.label}</h2>
          {group.items.map((action) => (
            <label className={styles.actionRow} data-done={action.done} key={action.id}>
              <input
                checked={action.done}
                onChange={(event) =>
                  send({
                    command: "note_action_toggle",
                    recording_id: note.recording_id,
                    action_id: action.id,
                    done: event.target.checked,
                  })
                }
                type="checkbox"
              />
              <span>
                <span className={styles.actionText}>{action.text}</span>
                {action.due != null && action.due !== "" && (
                  <small>{es.noteScreen.actionDue(action.due)}</small>
                )}
              </span>
            </label>
          ))}
        </section>
      ))}
    </section>
  );
}
