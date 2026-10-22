import { useState } from "react";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { NoteListItem } from "../../bridge/generated/NoteListItem";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Breadcrumbs, TopBarLeading } from "../../app/shell-slots";
import { Tabs } from "../../design/Tabs";
import { ViewError } from "../../design/ViewError";
import { es } from "../../i18n/es";
import { ActionsTab } from "./ActionsTab";
import { NoteHeader } from "./NoteHeader";
import { NoteProgress } from "./NoteProgress";
import type { NoteWriting } from "./note-writing";
import { type NoteTab, defaultTab } from "./note-tabs";
import { PrivateNotes } from "./PrivateNotes";
import type { Send } from "./send";
import { SummaryTab } from "./SummaryTab";
import { TranscriptTab } from "./TranscriptTab";
import styles from "./Note.module.css";

/** The tab the user chose for this note, or the one the note's state suggests. */
function useNoteTab(note: OpenNoteSnapshot): [NoteTab, (tab: NoteTab) => void] {
  const [chosen, setChosen] = useState<{ id: string; tab: NoteTab } | null>(null);
  const tab = chosen?.id === note.recording_id ? chosen.tab : defaultTab(note);
  return [tab, (next) => setChosen({ id: note.recording_id, tab: next })];
}

/** Resumen · Acciones · N · Transcripción · Notas privadas; the count once the note is written. */
function tabsOf(note: OpenNoteSnapshot): { id: NoteTab; label: string }[] {
  const open = note.actions.filter((action) => !action.done).length;
  return [
    { id: "summary", label: es.noteScreen.summaryTab },
    {
      id: "actions",
      label: note.summary ? es.noteScreen.actionsTabCount(open) : es.noteScreen.actionsTab,
    },
    { id: "transcript", label: es.noteScreen.transcriptTab },
    { id: "private", label: es.noteScreen.privateTab },
  ];
}

interface NoteParts {
  error: string | null;
  library?: NoteLibrarySnapshot | null;
  note: OpenNoteSnapshot;
  send: Send;
  /** What the note's editors offer: notes to link, people to mention, the principal. */
  writing?: NoteWriting;
}

/** The tab shown: its own part of the note. */
function NotePart({ error, library, note, send, tab, writing }: NoteParts & { tab: NoteTab }) {
  const key = note.recording_id;
  if (tab === "summary") {
    return <SummaryTab key={key} library={library} note={note} send={send} writing={writing} />;
  }
  if (tab === "actions") return <ActionsTab key={key} note={note} send={send} />;
  if (tab === "transcript") {
    return <TranscriptTab key={key} library={library} note={note} send={send} />;
  }
  return <PrivateNotes error={error} key={key} note={note} send={send} writing={writing} />;
}

/** One meeting: its header, how its note is coming along, and its parts. */
export function NoteDetail(
  props: NoteParts & {
    /** The meeting's row in the list: whether it was today or yesterday. */
    row?: NoteListItem;
  },
) {
  const { error, library, note, row, send } = props;
  const [tab, setTab] = useNoteTab(note);
  const ask = () => {
    void send({ command: "ask_orbit_about", recording_id: note.recording_id });
    window.location.hash = "#/assistant";
  };
  return (
    <div className={styles.page}>
      <TopBarLeading>
        <Breadcrumbs
          trail={[{ label: es.notes.section, href: "#/notes" }, { label: note.title }]}
        />
      </TopBarLeading>
      <article className={styles.read}>
        <ViewError error={error} />
        <NoteHeader library={library} note={note} onAsk={ask} row={row} send={send} />
        <NoteProgress note={note} send={send} />
        <div className={styles.tabs}>
          <Tabs
            label={es.noteScreen.partsLabel}
            onSelect={setTab}
            selected={tab}
            tabs={tabsOf(note)}
          >
            <NotePart {...props} tab={tab} />
          </Tabs>
        </div>
      </article>
    </div>
  );
}
