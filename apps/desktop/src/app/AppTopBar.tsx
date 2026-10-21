import type { AppView as AppSnapshot } from "../state/app-view";
import { useState } from "react";
import type { AppCommand } from "../bridge/generated";
import { currentHubConnection } from "../bridge/hub-state";
import { Pill } from "../design/Pill";
import { MeetingEntryPoints } from "../features/meetings/MeetingEntryPoints";
import { MeetingPill } from "../features/meetings/MeetingPill";
import type { ViewId } from "../state/ui";
import { titleForView } from "./Main";
import { settleHub, statusProblem } from "./status-problem";
import { TopBar } from "./TopBar";
import styles from "./TopBar.module.css";

interface AppTopBarProps {
  dispatch: (command: AppCommand) => void;
  navigate: (view: ViewId) => void;
  onOpenPalette: () => void;
  onRecord: () => void;
  onToggleTheme: () => void;
  snapshot: AppSnapshot;
  view: ViewId;
}

/** The hub link's last settled state, kept while it reconnects. */
function useSettledHub(snapshot: AppSnapshot) {
  const connection = currentHubConnection(snapshot);
  const [settled, setSettled] = useState(() => settleHub(null, connection));
  const next = settleHub(settled, connection);
  if (next !== settled) setSettled(next);
  return next;
}

/** Nothing when all is well; otherwise the problem and what to do. */
function ProblemPill({
  dispatch,
  navigate,
  snapshot,
}: Pick<AppTopBarProps, "dispatch" | "navigate" | "snapshot">) {
  const problem = statusProblem(snapshot, useSettledHub(snapshot));
  if (!problem) return null;
  const { action } = problem;
  return (
    <Pill status={problem.tone}>
      {problem.text}
      {action && (
        <button
          className={styles.pillAction}
          onClick={() => ("view" in action ? navigate(action.view) : dispatch(action.command))}
          type="button"
        >
          {action.label}
        </button>
      )}
    </Pill>
  );
}

export function AppTopBar({
  dispatch,
  navigate,
  onOpenPalette,
  onRecord,
  onToggleTheme,
  snapshot,
  view,
}: AppTopBarProps) {
  return (
    <TopBar
      meetEntry={<MeetingEntryPoints onRecord={onRecord} snapshot={snapshot} view={view} />}
      onOpenPalette={onOpenPalette}
      onToggleTheme={onToggleTheme}
      status={
        <>
          <ProblemPill dispatch={dispatch} navigate={navigate} snapshot={snapshot} />
          {snapshot.meeting.phase !== "idle" && <MeetingPill send={dispatch} snapshot={snapshot} />}
        </>
      }
      viewTitle={titleForView(view)}
    />
  );
}
