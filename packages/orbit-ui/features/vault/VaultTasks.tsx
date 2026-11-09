import { getCopy } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import { useState } from "react";
import type { VaultTask } from "../../bridge/generated/VaultTask";
import type { VaultMeta } from "../../bridge/generated/VaultMeta";
import { Button } from "../../design/Button";
import { Segmented } from "../../design/Segmented";

import { readFilter, type VaultPort } from "./api";
import { localToday, TaskRows, taskTitle } from "./TaskRows";
import { useVaultRead } from "./use-vault-read";
import styles from "./VaultReading.module.css";

type Grouping = "person" | "project";
interface Props {
  port?: VaultPort;
  notes: VaultMeta[];
  owner?: string;
  path?: string;
  query?: string;
  embedded?: boolean;
  initialState?: string;
}
function groupsOf(tasks: VaultTask[], by: Grouping, notes: VaultMeta[]) {
  const groups = new Map<string, VaultTask[]>();
  for (const task of tasks) {
    const group =
      by === "person"
        ? task.who || getCopy().vaultReading.unassigned
        : task.project === ""
          ? getCopy().vaultReading.noProject
          : taskTitle(task.project, notes);
    groups.set(group, [...(groups.get(group) ?? []), task]);
  }
  return [...groups];
}
function Filters({
  by,
  setBy,
  state,
  setState,
  owner,
  counts,
}: {
  by: Grouping;
  setBy: (by: Grouping) => void;
  state: string;
  setState: (state: string) => void;
  owner: string;
  counts: Partial<Record<string, number>> | undefined;
}) {
  const inlineCopy = useCopy().vaultReading;
  const copy = useCopy().extras.vaultCopy;
  const options = [
    ["open", inlineCopy.openFilter],
    ["mine", inlineCopy.mine],
    ["overdue", inlineCopy.overdueFilter],
    ["done", inlineCopy.doneFilter],
  ] as const;
  return (
    <>
      <header className={styles.heading}>
        <h1>{copy.tasks}</h1>
        <p>{copy.tasksLead}</p>
        <p className={styles.muted}>{copy.readOnly}</p>
      </header>
      <Segmented
        label={inlineCopy.groupTasks}
        onChange={setBy}
        options={[
          { value: "person", label: inlineCopy.byPerson },
          { value: "project", label: inlineCopy.byProject },
        ]}
        value={by}
      />
      <div className={styles.filters}>
        {options.map(([value, label]) => (
          <button
            aria-pressed={state === value}
            disabled={value === "mine" && owner === ""}
            key={value}
            onClick={() => setState(value)}
            type="button"
          >
            {label} {counts?.[value] ?? "…"}
          </button>
        ))}
      </div>
    </>
  );
}
function TaskGroups({
  data,
  by,
  embedded,
  notes,
}: {
  data: VaultTask[];
  by: Grouping;
  embedded: boolean;
  notes: VaultMeta[];
}) {
  return (
    <>
      {groupsOf(data, by, notes).map(([group, tasks]) => (
        <section key={group}>
          {!embedded && (
            <h2>
              {group} <small>{tasks.length}</small>
            </h2>
          )}
          <TaskRows notes={notes} tasks={tasks} />
        </section>
      ))}
    </>
  );
}

export function VaultTasks({
  port,
  notes,
  owner = "",
  path = "",
  query = "",
  embedded = false,
  initialState,
}: Props) {
  const inlineCopy = useCopy().vaultReading;
  const copy = useCopy().extras.vaultCopy;
  const [by, setBy] = useState<Grouping>("person");
  const [state, setState] = useState(initialState ?? (embedded ? "all" : "open"));
  const [number, setNumber] = useState(1);
  const filter = { ...readFilter("tasks", path, number, query), state, owner, today: localToday() };
  const { answer, error, retry } = useVaultRead(port, filter);
  const data = answer?.outcome === "tasks" ? answer.tasks : null;
  return (
    <section className={embedded ? styles.embedded : styles.content}>
      {!embedded && (
        <Filters
          by={by}
          counts={data?.counts}
          owner={owner}
          setBy={setBy}
          setState={(value) => {
            setState(value);
            setNumber(1);
          }}
          state={state}
        />
      )}
      {error !== null && (
        <p role="alert">
          {error}{" "}
          <Button onClick={retry} size={28}>
            {inlineCopy.retry}
          </Button>
        </p>
      )}
      {answer === null && error === null && <p role="status">{inlineCopy.readingTasks}</p>}
      {data !== null && data.tasks.length === 0 && <p>{inlineCopy.noTasks}</p>}
      <TaskGroups data={data?.tasks ?? []} by={by} embedded={embedded} notes={notes} />
      <div className={styles.actions}>
        {number > 1 && (
          <Button onClick={() => setNumber(number - 1)} size={28}>
            {inlineCopy.previous}
          </Button>
        )}
        {data?.next_page != null && (
          <Button onClick={() => setNumber(data.next_page ?? 1)} size={28}>
            {inlineCopy.moreTasks}
          </Button>
        )}
      </div>
      {!embedded && <p className={styles.muted}>{copy.tasksFooter}</p>}
    </section>
  );
}
