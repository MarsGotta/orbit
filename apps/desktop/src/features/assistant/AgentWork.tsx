import { useState, type ReactNode } from "react";
import type { AgentWork } from "../../bridge/generated/AgentWork";
import type { WorkItem } from "../../bridge/generated/WorkItem";
import type { WorkStepStatus } from "../../bridge/generated/WorkStepStatus";
import { Icon } from "../../design/Icon";
import { Spinner } from "../../design/Spinner";
import { useSpanish } from "../../i18n/AssistantCopy";
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

/**
 * The folded line of an ended turn: what its tools used, how many and how
 * long («Usó Mail · 3 pasos · 46 s»); without tools, only how long it worked.
 */
export function workSummary(work: AgentWork): string {
  const took =
    work.ended_at_ms == null ? null : durationText(work.ended_at_ms - work.started_at_ms);
  if (work.tools === 0) return took === null ? "" : copy.worked(took);
  const parts: string[] = [];
  if (work.subjects.length > 0) parts.push(copy.used(listText(work.subjects)));
  parts.push(copy.steps(work.tools));
  if (took !== null) parts.push(took);
  return parts.join(" · ");
}

const isStep = (item: WorkItem) => item.kind === "tool" || item.kind === "subagent";
const isRunning = (item: WorkItem) => item.phase === "started" || item.phase === "progress";

/** How the turn's wait for the user shows on its running step. */
export type StepWait = "approval" | "denied" | null;

function StepMark({ item, wait }: { item: WorkItem; wait: StepWait }) {
  if (item.phase === "completed")
    return (
      <span className={styles.stepDone}>
        <Icon name="check" size={14} />
      </span>
    );
  if (item.phase === "failed" || wait === "denied") return <span className={styles.stepNo}>×</span>;
  if (wait === "approval") return <span className={styles.stepWaiting} />;
  return <Spinner size={12} />;
}

/** Each step with its mark: ✓ done, the running one turning, × failed or refused. */
function Steps({ items, wait }: { items: WorkItem[]; wait: StepWait }) {
  const es = useSpanish();
  const steps = items.filter(isStep);
  if (steps.length === 0) return null;
  return (
    <ol aria-label={es.assistant.work.label} className={styles.steps}>
      {steps.map((item) => {
        const waits = isRunning(item) ? wait : null;
        return (
          <li
            data-current={waits === "approval" || undefined}
            data-phase={item.phase}
            key={item.id}
          >
            <span aria-hidden="true" className={styles.stepMark}>
              <StepMark item={item} wait={waits} />
            </span>
            <span>
              {item.label}
              {waits === "approval" && ` · ${es.assistant.stepWaits}`}
              {waits === "denied" && ` · ${es.assistant.stepDenied}`}
              {item.phase === "failed" && item.cause != null && (
                <span className={styles.workCause}> · {item.cause}</span>
              )}
            </span>
          </li>
        );
      })}
    </ol>
  );
}

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

/** The line above an answer: the agent's name and, after it, its work. */
export function AnswerHead({ children }: { children?: ReactNode }) {
  const es = useSpanish();
  return (
    <div className={styles.answerHead}>
      <strong>{es.chat.roles.assistant}</strong>
      {children !== undefined && children !== null && (
        <>
          <span aria-hidden="true">·</span>
          {children}
        </>
      )}
    </div>
  );
}

/**
 * What the agent did in a turn, on the line above its answer. While it works:
 * «Trabajando · 42 s», then each step with ✓ or the one running, its plan
 * and its reasoning, folded. Once it ends: one line, «Usó Mail · 3 pasos ·
 * 46 s», that unfolds every step.
 */
export function AgentWorkView({
  work,
  inFlight,
  wait = null,
}: {
  work: AgentWork;
  inFlight: boolean;
  wait?: StepWait;
}) {
  const now = useNow(inFlight ? 1_000 : null);
  const [open, setOpen] = useState(false);
  const plan = [...work.items].reverse().find((item) => item.kind === "plan");
  if (inFlight) {
    return (
      <>
        <AnswerHead>
          <span role="status">{copy.working(durationText(now - work.started_at_ms))}</span>
        </AnswerHead>
        <Steps items={work.items} wait={wait} />
        {plan && <Plan plan={plan} />}
        <Reasoning items={work.items} />
      </>
    );
  }
  const summary = workSummary(work);
  if (work.items.length === 0)
    return <AnswerHead>{summary === "" ? null : <span>{summary}</span>}</AnswerHead>;
  return (
    <>
      <AnswerHead>
        <button
          aria-expanded={open}
          className={styles.workToggle}
          onClick={() => setOpen(!open)}
          type="button"
        >
          {summary === "" ? copy.label : summary}
          <Icon name="chevron" size={14} />
        </button>
      </AnswerHead>
      {open && (
        <div className={styles.workDetail}>
          <Steps items={work.items} wait={wait} />
          {plan && <Plan plan={plan} />}
          <Reasoning items={work.items} />
        </div>
      )}
    </>
  );
}
