import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AppBootstrap, AppCommand, CommandReceipt, FeatureId } from "./generated";
import type { DaemonPort, FrameListener, Report, Unsubscribe } from "./port";
import type { AssistantCursor, MessagesPage, SessionPage } from "./reads";

const frameEvents: Record<FeatureId, string> = {
  status: "orbit://feature/status",
  permissions: "orbit://feature/permissions",
  agenda: "orbit://feature/agenda",
  notes: "orbit://feature/notes",
  note: "orbit://feature/note",
  recording: "orbit://feature/recording",
  assistant: "orbit://feature/assistant",
  control: "orbit://feature/control",
  briefing: "orbit://feature/briefing",
  local: "orbit://feature/local",
};

export class TauriPort implements DaemonPort {
  readSessionsPage(cursor: AssistantCursor | null): Promise<SessionPage> {
    return invoke<SessionPage>("orbit_read_sessions_page", { cursor });
  }

  readSessionMessages(sessionId: string, cursor: AssistantCursor | null): Promise<MessagesPage> {
    return invoke<MessagesPage>("orbit_read_session_messages", { sessionId, cursor });
  }
  snapshot(): Promise<AppBootstrap> {
    return invoke<AppBootstrap>("orbit_snapshot");
  }

  send(command: AppCommand, op: string): Promise<CommandReceipt> {
    return invoke<CommandReceipt>("orbit_send", { command, op });
  }

  report(report: Report): Promise<void> {
    return invoke<void>("orbit_report", { report });
  }

  async subscribeFrames(listener: FrameListener): Promise<Unsubscribe> {
    const unsubscribes = await Promise.all(
      Object.values(frameEvents).map((name) =>
        listen<Parameters<FrameListener>[0]>(name, (event) => listener(event.payload)),
      ),
    );
    return () => unsubscribes.forEach((unsubscribe) => unsubscribe());
  }
}
