import type { AppView as AppSnapshot } from "../state/app-view";
import { lazy as reactLazy, Suspense, useEffect } from "react";
import type { AppCommand } from "../bridge/generated";
import type { DaemonPort } from "../bridge/port";
import { es } from "../i18n/es";
import { useUiStore, type ViewId } from "../state/ui";
import styles from "./Main.module.css";
import { ViewBoundary } from "./ViewBoundary";

import BriefingView from "../features/briefing/BriefingView";

/** The loader of every view below that is loaded on demand. */
const lazyViewLoaders: Array<() => Promise<unknown>> = [];

/**
 * React's `lazy`, remembering the loader it was given so `loadLazyViews` runs
 * the very same import. Declaring a view with it is all it takes to be
 * covered: there is no second list of views to keep in step with this one.
 */
const lazy: typeof reactLazy = (load) => {
  lazyViewLoaders.push(load);
  return reactLazy(load);
};

const CalendarView = lazy(() => import("../features/calendar/CalendarView"));
const NotesView = lazy(() => import("../features/notes/NotesView"));
const AssistantView = lazy(() => import("../features/assistant/AssistantView"));
const SettingsView = lazy(() => import("../features/settings/SettingsView"));

/**
 * Loads the module of every lazy view now, instead of when each is first shown.
 *
 * The app never calls this: loading on demand is the point of `lazy`. The unit
 * suite does, before the first test of each file (`src/test-setup.ts`), so a test that renders one of these views waits for what it asks
 * about and not for an import racing the query's budget.
 */
export async function loadLazyViews(): Promise<void> {
  await Promise.all(lazyViewLoaders.map((load) => load()));
}

export function titleForView(view: ViewId): string {
  const titles: Record<ViewId, string> = {
    briefing: es.shell.briefing,
    calendar: es.shell.calendar,
    notes: es.shell.notes,
    assistant: es.shell.assistant,
    settings: es.shell.settings,
  };
  return titles[view];
}

export interface MainProps {
  /** A view that failed to paint: reported, and its words returned. */
  crash: () => string;
  error: string | null;
  onRecord: () => void;
  onReopenOnboarding: (mode?: "pair") => void;
  send: (command: AppCommand) => Promise<void>;
  snapshot: AppSnapshot;
  readSessionMessages?: DaemonPort["readSessionMessages"];
  agent?: DaemonPort["agent"];
  view: ViewId;
}

function LoadingView() {
  return <div className={styles.loading}>{es.shell.loadingView}</div>;
}

function View({
  error,
  onRecord,
  onReopenOnboarding,
  send,
  snapshot,
  readSessionMessages,
  agent,
  view,
}: MainProps) {
  if (view === "briefing")
    return <BriefingView error={error} onRecord={onRecord} send={send} snapshot={snapshot} />;
  if (view === "calendar") {
    return (
      <>
        <h1 className={styles.visuallyHidden}>{es.shell.calendar}</h1>
        <CalendarView error={error} send={send} snapshot={snapshot} />
      </>
    );
  }
  if (view === "notes") {
    return <NotesView error={error} onRecord={onRecord} send={send} snapshot={snapshot} />;
  }
  if (view === "assistant")
    return (
      <AssistantView
        error={error}
        readMessages={readSessionMessages}
        send={send}
        snapshot={snapshot}
      />
    );
  if (view === "settings") {
    return (
      <SettingsView
        agent={agent}
        error={error}
        onReopenOnboarding={onReopenOnboarding}
        send={send}
        snapshot={snapshot}
      />
    );
  }
  return <BriefingView error={error} onRecord={onRecord} send={send} snapshot={snapshot} />;
}

export function Main(props: MainProps) {
  const selectedNote = useUiStore((state) => state.selectedNote);
  const openRecordingId = props.snapshot.notes?.open?.recording_id;
  const send = props.send;
  const view = props.view;
  useEffect(() => {
    if (
      view === "notes" &&
      window.location.hash === "#/notes" &&
      selectedNote &&
      openRecordingId !== selectedNote.recordingId
    ) {
      void send({ command: "notes_open", recording_id: selectedNote.recordingId });
    }
  }, [openRecordingId, selectedNote, send, view]);
  return (
    <main className={styles.main}>
      <Suspense fallback={<LoadingView />}>
        <ViewBoundary key={view} onCrash={props.crash}>
          <View {...props} />
        </ViewBoundary>
      </Suspense>
    </main>
  );
}
