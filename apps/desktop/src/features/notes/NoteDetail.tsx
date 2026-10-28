import { getCopy } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import { useState } from "react";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { NoteListItem } from "../../bridge/generated/NoteListItem";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Breadcrumbs, TopBarLeading } from "../../app/shell-slots";
import { Tabs } from "../../design/Tabs";
import { ViewError } from "../../design/ViewError";

import { ActionsTab } from "./ActionsTab";
import { NoteHeader } from "./NoteHeader";
import { NoteProgress } from "./NoteProgress";
import type { NoteWriting } from "./note-writing";
import { type NoteTab, defaultTab } from "./note-tabs";
import { PrivateNotes } from "./PrivateNotes";
import type { Send } from "./send";
import { SummaryTab } from "./SummaryTab";
import { TranscriptTab } from "./TranscriptTab";
import { type Rewriting, useRewriting } from "./use-rewriting";
import styles from "./Note.module.css";

/** The tab the user chose for this note, or the one the note's state suggests. */
function useNoteTab(note: OpenNoteSnapshot): [NoteTab, (tab: NoteTab) => void] {
  const [chosen, setChosen] = useState<{ id: string; tab: NoteTab } | null>(null);
  const tab = chosen?.id === note.recording_id ? chosen.tab : defaultTab(note);
  return [tab, (next) => setChosen({ id: note.recording_id, tab: next })];
}

/** Summary · Actions · N · Transcript · Private notes; the count once the note is written. */
function tabsOf(note: OpenNoteSnapshot): { id: NoteTab; label: string }[] {
  const open = note.actions.filter((action) => !action.done).length;
  return [
    { id: "summary", label: getCopy().noteScreen.summaryTab },
    {
      id: "actions",
      label: note.summary
        ? getCopy().noteScreen.actionsTabCount(open)
        : getCopy().noteScreen.actionsTab,
    },
    { id: "transcript", label: getCopy().noteScreen.transcriptTab },
    { id: "private", label: getCopy().noteScreen.privateTab },
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
function NotePart({
  error,
  library,
  note,
  onRedoSummary,
  rewriting,
  send,
  tab,
  writing,
}: NoteParts & { onRedoSummary: () => void; rewriting: Rewriting; tab: NoteTab }) {
  const key = note.recording_id;
  if (tab === "summary") {
    return (
      <SummaryTab
        key={key}
        library={library}
        note={note}
        rewriting={rewriting}
        send={send}
        writing={writing}
      />
    );
  }
  if (tab === "actions") return <ActionsTab key={key} note={note} send={send} />;
  if (tab === "transcript") {
    return (
      <TranscriptTab
        key={key}
        library={library}
        note={note}
        onRedoSummary={onRedoSummary}
        send={send}
      />
    );
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
  const es = useCopy();
  const { error, library, note, row, send } = props;
  const [tab, setTab] = useNoteTab(note);
  const rewriting = useRewriting(note, send);
  // «Rehacer resumen» from a band under the transcript: the note is written
  // again and the Resumen shows how it goes.
  const redoSummary = () => {
    rewriting.redo();
    setTab("summary");
  };
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
        <NoteHeader
          library={library}
          note={note}
          onAsk={ask}
          rewriting={rewriting}
          row={row}
          send={send}
        />
        <NoteProgress note={note} send={send} />
        <div className={styles.tabs}>
          <Tabs
            label={es.noteScreen.partsLabel}
            onSelect={setTab}
            selected={tab}
            tabs={tabsOf(note)}
          >
            <NotePart {...props} onRedoSummary={redoSummary} rewriting={rewriting} tab={tab} />
          </Tabs>
        </div>
      </article>
    </div>
  );
}
