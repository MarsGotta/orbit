import type { AgentWork } from "../../bridge/generated/AgentWork";
import type { WorkItem } from "../../bridge/generated/WorkItem";
import type { WorkStepStatus } from "../../bridge/generated/WorkStepStatus";
import { es } from "../../i18n/es";
import { useNow } from "../../use-now";
import styles from "./AssistantView.module.css";

const copy = es.assistant.work;

/** «40 s», «1 min 12 s», «2 min»: as the macOS notification writes it. */
export function durationText(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1_000));
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  if (minutes === 0) return `${rest} s`;
  return rest === 0 ? `${minutes} min` : `${minutes} min ${rest} s`;
}

/** «a», «a y b», «a, b y c». */
function listText(subjects: string[]): string {
  if (subjects.length < 2) return subjects.join("");
  return `${subjects.slice(0, -1).join(", ")} y ${subjects[subjects.length - 1]}`;
}

/** The folded line of an ended turn: what its tools used, how many, how long. */
export function workSummary(work: AgentWork): string {
  const parts: string[] = [];
  if (work.tools > 0) {
    if (work.subjects.length > 0) parts.push(copy.used(listText(work.subjects)));
    parts.push(copy.steps(work.tools));
  }
  if (work.ended_at_ms != null)
    parts.push(copy.worked(durationText(work.ended_at_ms - work.started_at_ms)));
  return parts.join(" · ");
}

const running = (item: WorkItem) =>
  (item.kind === "tool" || item.kind === "subagent") &&
  (item.phase === "started" || item.phase === "progress");

function Plan({ plan }: { plan: WorkItem }) {
  const status: Record<WorkStepStatus, string> = copy.stepStatus;
  return (
    <ol aria-label={plan.label} className={styles.plan}>
      {plan.steps.map((step, index) => (
        <li data-status={step.status} key={index}>
          <span>{step.text}</span>
          <span className={styles.stepStatus}>{status[step.status]}</span>
        </li>
      ))}
    </ol>
  );
}

function Reasoning({ items }: { items: WorkItem[] }) {
  const said = items.filter((item) => item.kind === "reasoning" && item.label !== "");
  if (said.length === 0) return null;
  return (
    <details className={styles.reasoning}>
      <summary>{copy.reasoning}</summary>
      {said.map((item) => (
        <p key={item.id}>{item.label}</p>
      ))}
    </details>
  );
}

function Steps({ items }: { items: WorkItem[] }) {
  const steps = items.filter((item) => item.kind === "tool" || item.kind === "subagent");
  if (steps.length === 0) return null;
  return (
    <ul className={styles.workSteps}>
      {steps.map((item) => (
        <li data-phase={item.phase} key={item.id}>
          {item.label}
          {item.phase === "failed" && item.cause != null && (
            <span className={styles.workCause}> · {item.cause}</span>
          )}
        </li>
      ))}
    </ul>
  );
}

/**
 * What the agent did in a turn: while it works, a line per running tool,
 * its plan with each step's state and the time so far; once it ends, one
 * folded line that opens to every step. The reasoning, only its summary,
 * is folded in both.
 */
export function AgentWorkView({ work, inFlight }: { work: AgentWork; inFlight: boolean }) {
  const now = useNow(inFlight ? 1_000 : null);
  const plan = [...work.items].reverse().find((item) => item.kind === "plan");
  if (inFlight) {
    return (
      <section aria-label={copy.label} className={styles.work}>
        <p className={styles.workTime} role="status">
          {copy.working(durationText(now - work.started_at_ms))}
        </p>
        <ul className={styles.workSteps}>
          {work.items.filter(running).map((item) => (
            <li data-phase={item.phase} key={item.id}>
              {item.label}
            </li>
          ))}
        </ul>
        {plan && <Plan plan={plan} />}
        <Reasoning items={work.items} />
      </section>
    );
  }
  const summary = workSummary(work);
  if (work.items.length === 0) {
    return summary === "" ? null : (
      <section aria-label={copy.label} className={styles.work}>
        <p className={styles.workTime}>{summary}</p>
      </section>
    );
  }
  return (
    <section aria-label={copy.label} className={styles.work}>
      <details>
        <summary>{summary === "" ? copy.label : summary}</summary>
        <Steps items={work.items} />
        {plan && <Plan plan={plan} />}
        <Reasoning items={work.items} />
      </details>
    </section>
  );
}
