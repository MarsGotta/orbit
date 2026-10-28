import { settingsSectionKey } from "../state/ui";
import { useCopy } from "../i18n/CopyProvider";
import type { AppView as AppSnapshot } from "../state/app-view";
import { lazy as reactLazy, Suspense, useEffect } from "react";
import type { AppCommand } from "../bridge/generated";
import type { DaemonPort } from "../bridge/port";

import { routeFor, useUiStore, type SettingsSection, type ViewId } from "../state/ui";
import styles from "./Main.module.css";
import { Breadcrumbs, TopBarLeading } from "./shell-slots";
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
const VaultView = lazy(() => import("../features/vault/VaultView"));
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

export interface MainProps {
  /** A view that failed to paint: reported, and its words returned. */
  crash: () => string;
  error: string | null;
  onRecord: () => void;
  onReopenOnboarding: (mode?: "pair") => void;
  send: (command: AppCommand) => Promise<unknown>;
  snapshot: AppSnapshot;
  readSessionMessages?: DaemonPort["readSessionMessages"];
  readSessionsPage?: DaemonPort["readSessionsPage"];
  agent?: DaemonPort["agent"];
  vault?: DaemonPort["vault"];
  view: ViewId;
  settingsSection: SettingsSection;
}

function LoadingView() {
  const es = useCopy();
  return <div className={styles.loading}>{es.shell.loadingView}</div>;
}

function View({
  error,
  onRecord,
  onReopenOnboarding,
  send,
  snapshot,
  readSessionMessages,
  readSessionsPage,
  agent,
  vault,
  view,
  settingsSection,
}: MainProps) {
  const es = useCopy();
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
  if (view === "vault")
    return <VaultView owner={snapshot.settings.profile?.name ?? undefined} port={vault} />;
  if (view === "assistant") {
    const reads = { readMessages: readSessionMessages, readSessions: readSessionsPage };
    return <AssistantView agent={agent} error={error} send={send} snapshot={snapshot} {...reads} />;
  }
  if (view === "settings")
    return (
      <SettingsRoute
        {...{ agent, vault, error, onReopenOnboarding, send, snapshot, settingsSection }}
      />
    );
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

function SettingsRoute({
  agent,
  vault,
  error,
  onReopenOnboarding,
  send,
  snapshot,
  settingsSection,
}: Pick<
  MainProps,
  "agent" | "vault" | "error" | "onReopenOnboarding" | "send" | "snapshot" | "settingsSection"
>) {
  const es = useCopy();
  // SettingsView shows the section the route names (`#/settings/{section}`).
  // SettingsView's index opens a section through the router, so the URL and the index agree.
  const onSection = (section: SettingsSection) => useUiStore.getState().openSettings(section);
  const settings = {
    agent,
    vault,
    error,
    onReopenOnboarding,
    send,
    snapshot,
    section: settingsSection,
    onSection,
  };
  return (
    <>
      <TopBarLeading>
        <Breadcrumbs
          trail={[
            { label: es.shell.settings, href: routeFor("settings") },
            { label: es.shell.settingsSections[settingsSectionKey(settingsSection)] },
          ]}
        />
      </TopBarLeading>
      <SettingsView {...settings} />
    </>
  );
}
