import type { AppView as AppSnapshot } from "../state/app-view";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { AppCommand } from "../bridge/generated";
import type { DaemonPort } from "../bridge/port";
import { CommandPalette, type CommandItem } from "../design/CommandPalette";
import { es } from "../i18n/es";
import { AssistantCopy } from "../i18n/AssistantCopy";
import { MeetingOverlays } from "../features/meetings/MeetingOverlays";
import { onboardingWasCompleted } from "../features/onboarding/OnboardingView";
import { initializeUiStore, useUiStore, type ViewId } from "../state/ui";
import { messageFor } from "./visible-error";
import { AppNotices } from "./AppNotices";
import { AppOnboarding } from "./AppOnboarding";
import { useAppStore } from "./use-app-store";
import { AppTopBar } from "./AppTopBar";
import { useErrorSelection } from "./error-selection";
import { AppSidebar } from "./AppSidebar";
import { Main } from "./Main";
import type { Message } from "./interceptor";
import { ViewBoundary } from "./ViewBoundary";
import { reportingReads } from "./report-failure";
import type { OnboardingRequest } from "./Onboarding";
import styles from "./App.module.css";
import { usePaletteCommands } from "./use-palette-commands";
import { useScopedSend } from "./use-scoped-send";

export interface AppContentProps {
  commands: CommandItem[];
  crash: () => string;
  dismiss: (op: string) => void;
  error: string | null;
  messages: Message[];
  recordingError: string | null;
  meetingOpen: boolean;
  onboardingError: string | null;
  navigate: (view: ViewId) => void;
  paletteOpen: boolean;
  send: (command: AppCommand) => Promise<void>;
  sendOnboarding: (command: AppCommand) => Promise<void>;
  sendRecording: (command: AppCommand) => Promise<void>;
  setMeetingOpen: (open: boolean) => void;
  setPaletteOpen: (open: boolean) => void;
  snapshot: AppSnapshot;
  readSessionsPage?: DaemonPort["readSessionsPage"];
  readSessionMessages?: DaemonPort["readSessionMessages"];
  theme: string;
  toggleTheme: () => void;
  view: ViewId;
}

function useCommandDispatch(send: AppContentProps["send"]): (command: AppCommand) => void {
  return useCallback(
    (command: AppCommand) => {
      if (command.command === "notes_open") {
        const [recordingId] = command.recording_id.split("?", 1);
        if (recordingId) void send({ ...command, recording_id: recordingId });
        return;
      }
      void send(command);
    },
    [send],
  );
}

function AppContent(props: AppContentProps) {
  const [onboardingDone, setOnboardingDone] = useState(onboardingWasCompleted);
  const [onboardingRequested, setOnboardingRequested] = useState<OnboardingRequest>(null);
  const recordingMessage = messageFor("recording", props.messages);
  const dispatch = useCommandDispatch(props.send);
  const recordingDispatch = useCommandDispatch(props.sendRecording);
  const displayView = window.location.hash.startsWith("#/notes/") ? "notes" : props.view;
  return (
    <div className={`${styles.app} ${props.theme === "light" ? "o-light" : ""}`}>
      <AppTopBar
        dispatch={recordingDispatch}
        onOpenPalette={() => props.setPaletteOpen(true)}
        onRecord={() => props.setMeetingOpen(true)}
        navigate={props.navigate}
        onToggleTheme={props.toggleTheme}
        snapshot={props.snapshot}
        view={displayView}
      />
      <AppNotices {...props} />
      <div className={styles.body} data-app-content>
        <AppSidebar dispatch={dispatch} props={props} view={displayView} />
        <Main
          crash={props.crash}
          error={props.error}
          onRecord={() => props.setMeetingOpen(true)}
          onReopenOnboarding={(mode) => setOnboardingRequested(mode ?? "full")}
          send={props.send}
          snapshot={props.snapshot}
          readSessionMessages={props.readSessionMessages}
          view={displayView}
        />
      </div>
      <ViewBoundary onCrash={props.crash}>
        <MeetingOverlays
          error={props.recordingError}
          errorKind={recordingMessage?.class}
          onErrorDismiss={() => recordingMessage && props.dismiss(recordingMessage.op)}
          launcherOpen={props.meetingOpen}
          onLauncherClose={() => props.setMeetingOpen(false)}
          send={recordingDispatch}
          snapshot={props.snapshot}
        />
      </ViewBoundary>
      <CommandPalette
        commands={props.commands}
        onOpenChange={props.setPaletteOpen}
        open={props.paletteOpen}
      />
      <ViewBoundary onCrash={props.crash}>
        <AppOnboarding
          done={onboardingDone}
          props={{ ...props, send: props.sendOnboarding, error: props.onboardingError }}
          requested={onboardingRequested}
          setDone={setOnboardingDone}
          setRequested={setOnboardingRequested}
        />
      </ViewBoundary>
    </div>
  );
}

export function App({ port }: { port: DaemonPort }) {
  // What a read could not read is reported; the screen only says so.
  const { readSessionsPage, readSessionMessages } = useMemo(() => reportingReads(port), [port]);
  const { store, snapshot, error, send, messages, dismiss, crash } = useAppStore(port);
  const { theme, view, navigate, toggleTheme } = useUiStore();
  const { errorView, visibleError, onboardingError, recordingError } = useErrorSelection(
    store,
    view,
  );
  const {
    fromView: sendFromView,
    fromOnboarding: sendOnboarding,
    fromRecording: sendRecording,
  } = useScopedSend(send, errorView);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [meetingOpen, setMeetingOpen] = useState(false);
  const commands = usePaletteCommands(navigate, sendFromView, send, setMeetingOpen);

  useEffect(() => initializeUiStore(), []);

  if (!snapshot) {
    return (
      <>
        <main className={styles.loading}>{error ?? es.loading}</main>
        <CommandPalette commands={commands} onOpenChange={setPaletteOpen} open={paletteOpen} />
      </>
    );
  }
  return (
    <AssistantCopy name={snapshot.settings.assistant_name}>
      <AppContent
        commands={commands}
        crash={crash}
        dismiss={dismiss}
        error={visibleError ?? error}
        messages={messages}
        onboardingError={onboardingError ?? error}
        recordingError={recordingError}
        meetingOpen={meetingOpen}
        navigate={navigate}
        paletteOpen={paletteOpen}
        send={sendFromView}
        sendOnboarding={sendOnboarding}
        sendRecording={sendRecording}
        setMeetingOpen={setMeetingOpen}
        setPaletteOpen={setPaletteOpen}
        snapshot={snapshot}
        readSessionsPage={readSessionsPage}
        readSessionMessages={readSessionMessages}
        theme={theme}
        toggleTheme={toggleTheme}
        view={view}
      />
    </AssistantCopy>
  );
}
