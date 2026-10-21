import { useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { ViewError } from "../../design/ViewError";
import { es } from "../../i18n/es";
import { ActionsTab } from "./ActionsTab";
import { formatDate } from "./notes-format";
import { NoteHeader } from "./NoteHeader";
import { NoteProgress } from "./NoteProgress";
import { type NoteTab, defaultTab } from "./note-tabs";
import { PrivateNotes } from "./PrivateNotes";
import { SummaryTab } from "./SummaryTab";
import { TranscriptTab, TranscriptionProgress } from "./TranscriptTab";
import listStyles from "./NotesView.module.css";
import styles from "./NoteScreen.module.css";

type Send = (command: AppCommand) => void;

const tabs: Array<[NoteTab, string]> = [
  ["private", es.notes.privateTab],
  ["summary", es.notes.summaryTab],
  ["actions", es.notes.actionsTab],
  ["transcript", es.notes.transcriptTab],
];

/** The tab the user chose for this note, or the one the note's state suggests. */
function useNoteTab(note: OpenNoteSnapshot): [NoteTab, (tab: NoteTab) => void] {
  const [chosen, setChosen] = useState<{ id: string; tab: NoteTab } | null>(null);
  const tab = chosen?.id === note.recording_id ? chosen.tab : defaultTab(note);
  return [tab, (next) => setChosen({ id: note.recording_id, tab: next })];
}

function NoteTabs({ onSelect, selected }: { onSelect: (tab: NoteTab) => void; selected: NoteTab }) {
  return (
    <div aria-label={es.shell.notes} className={styles.tabs} role="tablist">
      {tabs.map(([id, label]) => (
        <button
          aria-selected={selected === id}
          key={id}
          onClick={() => onSelect(id)}
          role="tab"
          type="button"
        >
          {label}
        </button>
      ))}
    </div>
  );
}

function NotePanel({
  note,
  send,
  tab,
  library,
}: {
  note: OpenNoteSnapshot;
  send: Send;
  tab: NoteTab;
  library?: NoteLibrarySnapshot | null;
}) {
  return (
    <div className={styles.tabPanel} role="tabpanel">
      {tab === "private" && <PrivateNotes key={note.recording_id} note={note} send={send} />}
      {tab === "summary" && <SummaryTab key={note.recording_id} note={note} send={send} />}
      {tab === "actions" && <ActionsTab note={note} send={send} />}
      {tab === "transcript" && (
        <TranscriptTab key={note.recording_id} note={note} send={send} library={library} />
      )}
    </div>
  );
}

/** The note's bar: where it is, its progress in the middle, and how old
 * the data on screen is at the end. */
function NoteBar({ note, series }: { note: OpenNoteSnapshot; series: string }) {
  return (
    <header className={listStyles.noteBar}>
      <span>
        {series} <span aria-hidden="true">›</span>
      </span>
      <time>{formatDate(note.started_at_ms)}</time>
      <NoteProgress progress={note.progress} />
    </header>
  );
}

export function NoteDetail({
  error,
  library,
  note,
  send,
  series,
}: {
  error: string | null;
  library?: NoteLibrarySnapshot | null;
  note: OpenNoteSnapshot;
  send: Send;
  series: string;
}) {
  const [tab, setTab] = useNoteTab(note);
  const ask = () => {
    send({ command: "ask_orbit_about", recording_id: note.recording_id });
    window.location.hash = "#/assistant";
  };
  return (
    <section className={listStyles.detail}>
      <ViewError error={error} />
      <NoteBar note={note} series={series} />
      <div className={listStyles.noteBody}>
        <NoteHeader library={library} note={note} onAsk={ask} send={send} />
        <TranscriptionProgress note={note} send={send} />
        <NoteTabs onSelect={setTab} selected={tab} />
        <NotePanel note={note} send={send} tab={tab} library={library} />
      </div>
    </section>
  );
}
