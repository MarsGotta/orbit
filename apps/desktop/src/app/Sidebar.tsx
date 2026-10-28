import type { AppCommand } from "../bridge/generated";
import { BrandMark } from "../design/Brand";
import { RecordButton } from "../design/Button";
import { SidebarItem } from "../design/SidebarItem";
import { agentFor } from "../features/assistant/agents";
import { noteRowState } from "../features/notes/note-row";
import { useCopy } from "../i18n/CopyProvider";
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
  /** Its answer is the daemon's: `false` when it refused the order. */
  send: (command: AppCommand) => unknown;
}

/** Meetings whose recording or note is on its way: recording, uploading, transcribing, writing. */
function inProgress(snapshot: AppSnapshot): number {
  const notes = snapshot.notes?.list ?? [];
  return notes.filter((note) => {
    const tone = noteRowState(note).tone;
    return tone === "live" || tone === "working";
  }).length;
}

/** An assistant is working on an answer: thinking it or writing it. */
function assistantWorking(snapshot: AppSnapshot): boolean {
  const activity = snapshot.chat.activity;
  return activity != null && activity !== "transcribing";
}

/** While a recording or a call goes on, its card takes the place of «Grabar reunión». */
function Live({ onRecord, send, snapshot }: Omit<SidebarProps, "view">) {
  const es = useCopy();
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
 * reunión»), the four sections by name, Settings and the person.
 */
export function Sidebar(props: SidebarProps) {
  const es = useCopy();
  const { snapshot, view } = props;
  const working = inProgress(snapshot);
  const name = snapshot.settings.profile?.name?.trim() || es.shell.localUser;
  return (
    <nav aria-label={es.shell.viewsLabel} className={styles.sidebar}>
      <div aria-hidden="true" className={styles.windowButtons} />
      <div className={styles.logo}>
        <BrandMark orbiting={assistantWorking(snapshot)} />
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
          icon="meetings"
          label={es.shell.notes}
          meta={working > 0 ? es.shell.inProgress(working) : undefined}
        />
        <SidebarItem
          active={view === "vault"}
          href={routeFor("vault")}
          icon="notes"
          label={es.shell.vault}
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
      <Person name={name} />
    </nav>
  );
}

function Person({ name }: { name: string }) {
  return (
    <div className={styles.person}>
      <span aria-hidden="true" className={styles.initial}>
        {name.charAt(0).toUpperCase()}
      </span>
      <span className={styles.name}>{name}</span>
    </div>
  );
}
