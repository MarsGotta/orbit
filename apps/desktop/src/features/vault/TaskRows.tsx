import type { VaultMeta } from "../../bridge/generated/VaultMeta";
import type { VaultTask } from "../../bridge/generated/VaultTask";
import { noteRoute } from "./route";
import styles from "./VaultReading.module.css";

export function taskTitle(path: string, notes: VaultMeta[]) {
  return (
    notes.find((note) => note.path === path.replace(/^\//, ""))?.title ??
    path.split("/").pop()?.replace(/\.md$/, "").replace(/-/g, " ") ??
    path
  );
}
export function localToday() {
  const date = new Date();
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}
export function displayDate(value: string) {
  const date = new Date(value.length === 10 ? `${value}T12:00:00` : value);
  return Number.isNaN(date.getTime())
    ? value
    : new Intl.DateTimeFormat("es-ES", { day: "numeric", month: "short" }).format(date);
}

export function TaskRows({ tasks, notes }: { tasks: VaultTask[]; notes: VaultMeta[] }) {
  return (
    <ul className={styles.tasks}>
      {tasks.map((task) => (
        <li
          className={styles.task}
          id={`vault-task-${task.block_id || task.id}`}
          key={`${task.file}:${task.line}`}
        >
          <input
            aria-label={`${task.done ? "Hecha" : "Abierta"}: ${task.text}`}
            checked={task.done}
            disabled
            readOnly
            title="Solo lectura: marcar tareas llegará más adelante"
            type="checkbox"
          />
          <div className={styles.taskText}>
            <a data-done={task.done} href={noteRoute(task.file)}>
              {task.text}
            </a>
            <small>
              {task.priority !== "" && <>{task.priority} · </>}
              {task.scheduled !== "" && <>Programada {displayDate(task.scheduled)} · </>}
              {task.start !== "" && <>Inicio {displayDate(task.start)} · </>}
              {task.recurrence !== "" && <>🔁 {task.recurrence} · </>}
              {task.done_at !== "" && <>Hecha el {displayDate(task.done_at)} · </>}
              {task.who !== "" && <>{task.who} · </>}
              {task.project !== "" && <>en {taskTitle(task.project, notes)} · </>}
              {task.source !== "" && (
                <>
                  de <a href={noteRoute(task.source)}>{taskTitle(task.source, notes)}</a>
                </>
              )}
            </small>
          </div>
          <span
            className={styles.due}
            data-late={!task.done && task.due !== "" && task.due < localToday()}
          >
            {task.done
              ? "Hecha"
              : task.due === ""
                ? "Sin fecha"
                : `${task.due < localToday() ? "Vencida · " : ""}${displayDate(task.due)}`}
          </span>
          <span aria-hidden="true" className={styles.avatar}>
            {task.who.charAt(0) || "–"}
          </span>
        </li>
      ))}
    </ul>
  );
}
