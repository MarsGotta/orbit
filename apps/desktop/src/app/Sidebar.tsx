import type { AppCommand } from "../bridge/generated";
import { RecordButton } from "../design/Button";
import { Icon } from "../design/Icon";
import { SidebarItem } from "../design/SidebarItem";
import { agentFor } from "../features/assistant/agents";
import { noteRowState } from "../features/notes/note-row";
import { useSpanish } from "../i18n/AssistantCopy";
import type { AppView as AppSnapshot } from "../state/app-view";
import { routeFor, type ViewId } from "../state/ui";
import { CallCard } from "./CallCard";
import { RecordingCard } from "./RecordingCard";
import styles from "./Sidebar.module.css";

export interface SidebarProps {
  snapshot: AppSnapshot;
  view: ViewId;
  /** Opens «Nueva grabación». */
  onRecord: () => void;
  send: (command: AppCommand) => void;
}

/** Meetings whose recording or note is on its way: recording, uploading, transcribing, writing. */
function inProgress(snapshot: AppSnapshot): number {
  const notes = snapshot.notes?.list ?? [];
  return notes.filter((note) => {
    const tone = noteRowState(note).tone;
    return tone === "live" || tone === "working";
  }).length;
}

/** While a recording or a call goes on, its card takes the place of «Grabar reunión». */
function Live({ onRecord, send, snapshot }: Omit<SidebarProps, "view">) {
  const es = useSpanish();
  const recording = snapshot.meeting.phase !== "idle";
  const call = snapshot.call;
  if (!recording && !call)
    return (
      <RecordButton className={styles.record} onClick={onRecord} shortcut={es.shell.recordShortcut}>
        {es.shell.record}
      </RecordButton>
    );
  const agent = agentFor(snapshot, call?.agent_id)?.name ?? es.shell.assistantName;
  return (
    <>
      <RecordingCard meeting={snapshot.meeting} send={send} timezone={snapshot.system_timezone} />
      {call && <CallCard agentName={agent} call={call} send={send} />}
    </>
  );
}

/**
 * The sidebar, 224 px, in every view: Orbit, what is going on (or «Grabar
 * reunión»), the four sections by name, Ajustes and the person.
 */
export function Sidebar(props: SidebarProps) {
  const es = useSpanish();
  const { snapshot, view } = props;
  const working = inProgress(snapshot);
  const name = snapshot.settings.profile?.name?.trim() || es.shell.localUser;
  return (
    <nav aria-label={es.shell.viewsLabel} className={styles.sidebar}>
      <div aria-hidden="true" className={styles.windowButtons} />
      <div className={styles.logo}>
        <Icon name="logo" size={20} />
        <span>{es.appName}</span>
      </div>
      <Live onRecord={props.onRecord} send={props.send} snapshot={snapshot} />
      <div className={styles.items}>
        <SidebarItem
          active={view === "briefing"}
          href={routeFor("briefing")}
          icon="briefing"
          label={es.shell.briefing}
        />
        <SidebarItem
          active={view === "calendar"}
          href={routeFor("calendar")}
          icon="calendar"
          label={es.shell.calendar}
        />
        <SidebarItem
          active={view === "notes"}
          href={routeFor("notes")}
          icon="notes"
          label={es.shell.notes}
          meta={working > 0 ? es.shell.inProgress(working) : undefined}
        />
        <SidebarItem
          active={view === "assistant"}
          alert={snapshot.chat.pending ? { tone: "rec", text: es.shell.needsYou } : undefined}
          href={routeFor("assistant")}
          icon="assistant"
          label={es.shell.assistant}
        />
      </div>
      <span className={styles.spacer} />
      <SidebarItem
        active={view === "settings"}
        href={routeFor("settings")}
        icon="settings"
        label={es.shell.settings}
      />
      <div className={styles.person}>
        <span aria-hidden="true" className={styles.initial}>
          {name.charAt(0).toUpperCase()}
        </span>
        <span className={styles.name}>{name}</span>
      </div>
    </nav>
  );
}
