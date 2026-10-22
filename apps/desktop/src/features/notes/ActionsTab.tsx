import type { AppCommand } from "../../bridge/generated";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";
import type { NoteActionSnapshot } from "../../bridge/generated/NoteActionSnapshot";
import { es } from "../../i18n/es";
import { useHeldChoice } from "../../state/use-held-choice";
import { actionsMarkdown, groupActions } from "./action-groups";
import styles from "./NoteScreen.module.css";

/** The tasks of the note, grouped by who owns them. */
export function ActionsTab({
  note,
  send,
}: {
  note: OpenNoteSnapshot;
  send: (command: AppCommand) => unknown;
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
          size={28}
          variant="ghost"
        >
          {es.noteScreen.copyActions}
        </Button>
      </div>
      {groups.map((group) => (
        <section className={styles.actionGroup} data-mine={group.mine} key={group.key}>
          <h2>{group.label}</h2>
          {group.items.map((action) => (
            <ActionRow
              action={action}
              key={action.id}
              recordingId={note.recording_id}
              send={send}
            />
          ))}
        </section>
      ))}
    </section>
  );
}

/** One task, its box kept as ticked until the note is read again with it. */
function ActionRow({
  action,
  recordingId,
  send,
}: {
  action: NoteActionSnapshot;
  recordingId: string;
  send: (command: AppCommand) => unknown;
}) {
  const done = useHeldChoice(action.done, send);
  return (
    <label className={styles.actionRow} data-done={done.shown}>
      <input
        checked={done.shown}
        onChange={(event) =>
          done.choose(event.target.checked, {
            command: "note_action_toggle",
            recording_id: recordingId,
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
  );
}
