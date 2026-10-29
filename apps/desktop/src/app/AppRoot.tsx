import type { AppView as AppSnapshot } from "../state/app-view";
import { useEffect, useMemo, useState } from "react";
import type { AppCommand } from "../bridge/generated";
import type { DaemonPort } from "../bridge/port";
import { AssistantSessions } from "../features/assistant/AssistantSessions";
import { onboardingWasCompleted } from "../features/onboarding/OnboardingView";
import { initializeUiStore, useUiStore, type SettingsSection, type ViewId } from "../state/ui";
import { NotResponding, Starting } from "./AppStates";
import type { useAppStore } from "./use-app-store";
import { useErrorSelection } from "./error-selection";
import { Main } from "./Main";
import type { Message } from "./interceptor";
import { reportingAgent, reportingReads } from "./report-failure";
import type { OnboardingRequest } from "./Onboarding";
import { Overlays, useCommandDispatch } from "./AppOverlays";
import { Palette } from "./Palette";
import { ShellSlots, useShellSlots } from "./shell-slots";
import { Sidebar } from "./Sidebar";
import { TopBar } from "./TopBar";
import styles from "./Shell.module.css";
import { usePaletteCommands, type PaletteContent } from "./use-palette-commands";
import { FailureActionsProvider } from "./FailureActions";
import { useScopedSend } from "./use-scoped-send";
import { OrbitStoreProvider } from "../state/use-in-flight";
import { flushBeforeQuit } from "../state/quit-flush";
import { useWindowPreferences } from "../state/window-preferences";
import { useWindowMenu } from "./use-window-menu";

export interface AppContentProps {
  palette: PaletteContent;
  crash: () => string;
  dismiss: (op: string) => void;
  /** «Reintentar» on a failure: its order again, from where it was sent. */
  retry: (op: string) => void;
  error: string | null;
  messages: Message[];
  recordingError: string | null;
  meetingOpen: boolean;
  onboardingError: string | null;
  navigate: (view: ViewId) => void;
  paletteOpen: boolean;
  send: (command: AppCommand) => Promise<unknown>;
  sendOnboarding: (command: AppCommand) => Promise<unknown>;
  sendRecording: (command: AppCommand) => Promise<unknown>;
  setMeetingOpen: (open: boolean) => void;
  setPaletteOpen: (open: boolean) => void;
  snapshot: AppSnapshot;
  readSessionsPage?: DaemonPort["readSessionsPage"];
  readSessionMessages?: DaemonPort["readSessionMessages"];
  agent?: DaemonPort["agent"];
  vault?: DaemonPort["vault"];
  view: ViewId;
  settingsSection: SettingsSection;
}

/** Where a view's list goes: the column beside the sidebar. */
function ColumnHost() {
  return <div className={styles.columnHost} ref={useShellSlots()?.setColumn} />;
}

function AppContent(props: AppContentProps) {
  const sidebar = useWindowPreferences((state) => state.sidebar);
  const [done, setDone] = useState(onboardingWasCompleted);
  const [requested, setRequested] = useState<OnboardingRequest>(null);
  const dispatch = useCommandDispatch(props.send);
  const recordingDispatch = useCommandDispatch(props.sendRecording);
  const onRecord = () => props.setMeetingOpen(true);
  return (
    <ShellSlots>
      <div className={styles.app} data-app-content data-sidebar-hidden={!sidebar || undefined}>
        {sidebar && (
          <Sidebar
            onRecord={onRecord}
            send={recordingDispatch}
            snapshot={props.snapshot}
            view={props.view}
          />
        )}
        <ColumnHost />
        <div className={styles.mainColumn}>
          <TopBar
            onOpenPalette={() => props.setPaletteOpen(true)}
            onPair={() => setRequested("pair")}
            send={props.send}
            snapshot={props.snapshot}
          />
          <FailureActionsProvider messages={props.messages} onRetry={props.retry}>
            <Main
              agent={props.agent}
              vault={props.vault}
              crash={props.crash}
              error={props.error}
              onRecord={onRecord}
              onReopenOnboarding={(mode) => setRequested(mode ?? "full")}
              readSessionMessages={props.readSessionMessages}
              readSessionsPage={props.readSessionsPage}
              send={props.send}
              settingsSection={props.settingsSection}
              snapshot={props.snapshot}
              view={props.view}
            />
          </FailureActionsProvider>
        </div>
        {props.view === "assistant" && (
          <AssistantSessions
            read={props.readSessionsPage}
            send={dispatch}
            snapshot={props.snapshot}
          />
        )}
      </div>
      <Overlays {...props} onboarding={{ done, setDone, requested, setRequested }} />
    </ShellSlots>
  );
}

/** The main window answers Orbit.app's ask before it quits, for as long as it is open. */
function useQuitFlush(port: DaemonPort) {
  useEffect(() => {
    let active = true;
    let stop: () => void = () => undefined;
    void port.onFlushBeforeQuit?.(flushBeforeQuit).then(
      (unsubscribe) => {
        if (active) stop = unsubscribe;
        else unsubscribe();
      },
      () => undefined,
    );
    return () => {
      active = false;
      stop();
    };
  }, [port]);
}

export function AppRoot({ port, app }: { port: DaemonPort; app: ReturnType<typeof useAppStore> }) {
  const { store, snapshot, error, send, messages, dismiss, retry, crash } = app;
  // What a read could not read is reported; the screen only says so. Each
  // read is followed by the registry of what is in flight.
  const { readSessionsPage, readSessionMessages, agent } = useReportingPorts(port, store);
  const { view, navigate, settingsSection } = useUiStore();
  const { errorView, visibleError, onboardingError, recordingError } = useErrorSelection(
    store,
    view,
  );
  const { fromView, fromOnboarding, fromRecording } = useScopedSend(send, errorView);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [meetingOpen, setMeetingOpen] = useState(false);
  const palette = usePaletteCommands(navigate, send, setMeetingOpen, snapshot);
  // Closing Orbit is Orbit.app's own order: it works while the daemon does not answer.
  const quit = () => void send({ command: "quit" });

  useWindowLifecycle(port);
  useWindowMenu(port, snapshot, fromRecording, () => setMeetingOpen(true));

  if (!snapshot && error) return <NotResponding onClose={quit} />;
  if (!snapshot)
    return (
      <>
        <Starting onClose={quit} />
        <Palette {...palette} onOpenChange={setPaletteOpen} open={paletteOpen} />
      </>
    );
  if (snapshot.connection.daemon === "lost") return <NotResponding onClose={quit} />;
  return (
    <OrbitStoreProvider store={store}>
      <AppContent
        agent={agent}
        vault={port.vault}
        palette={palette}
        crash={crash}
        dismiss={dismiss}
        retry={retry}
        error={visibleError ?? error}
        meetingOpen={meetingOpen}
        messages={messages}
        navigate={navigate}
        onboardingError={onboardingError ?? error}
        paletteOpen={paletteOpen}
        readSessionMessages={readSessionMessages}
        readSessionsPage={readSessionsPage}
        recordingError={recordingError}
        send={fromView}
        sendOnboarding={fromOnboarding}
        sendRecording={fromRecording}
        setMeetingOpen={setMeetingOpen}
        setPaletteOpen={setPaletteOpen}
        settingsSection={settingsSection}
        snapshot={snapshot}
        view={view}
      />
    </OrbitStoreProvider>
  );
}

function useReportingPorts(port: DaemonPort, store: ReturnType<typeof useAppStore>["store"]) {
  return useMemo(
    () => ({
      ...reportingReads(port, store.getState().track),
      agent: reportingAgent(port, store.getState().track),
    }),
    [port, store],
  );
}
function useWindowLifecycle(port: DaemonPort) {
  useEffect(() => initializeUiStore(), []);
  useQuitFlush(port);
}
