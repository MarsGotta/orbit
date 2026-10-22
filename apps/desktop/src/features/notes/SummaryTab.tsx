import { type CSSProperties, useRef, useState } from "react";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import type { Section } from "../../bridge/generated/Section";
import { Button } from "../../design/Button";
import { Skeleton } from "../../design/Skeleton";
import { es } from "../../i18n/es";
import { notRequested, templateOf } from "./note-progress";
import { shortName } from "./notes-format";
import type { Send } from "./send";
import type { NoteWriting } from "./note-writing";
import { SummarySection } from "./SummarySection";
import { SummaryTools } from "./SummaryTools";
import { useDebouncedCommit } from "./use-debounced-commit";
import type { Rewriting } from "./use-rewriting";
import styles from "./Note.module.css";

/** «Quién habló»: one bar, each person's share in their colour. */
function WhoSpoke({ note }: { note: OpenNoteSnapshot }) {
  const speakers = note.speakers.filter((speaker) => speaker.share_pct > 0);
  if (speakers.length === 0) return null;
  return (
    <section className={styles.whoSpoke}>
      <h3>{es.noteScreen.whoSpoke}</h3>
      <div aria-hidden="true" className={styles.shareBar}>
        {speakers.map((speaker) => (
          <span
            key={speaker.key}
            style={{ width: `${speaker.share_pct}%`, background: speaker.color } as CSSProperties}
          />
        ))}
      </div>
      <ul className={styles.shareLegend}>
        {speakers.map((speaker) => (
          <li key={speaker.key}>{es.noteScreen.share(shortName(speaker), speaker.share_pct)}</li>
        ))}
      </ul>
    </section>
  );
}

function List({ items, title }: { items: string[]; title: string }) {
  if (items.length === 0) return null;
  return (
    <section>
      <h3>{title}</h3>
      <ul>
        {items.map((item, index) => (
          <li key={index}>{item}</li>
        ))}
      </ul>
    </section>
  );
}

function Superseded({ note, send }: { note: OpenNoteSnapshot; send: Send }) {
  if (note.summary?.superseded_available !== true) return null;
  return (
    <p className={styles.band}>
      <span>{es.noteScreen.supersededAvailable}</span>
      <Button
        onClick={() =>
          void send({ command: "note_restore_superseded", recording_id: note.recording_id })
        }
        size={28}
        variant="ghost"
      >
        {es.noteScreen.restoreSuperseded}
      </Button>
    </p>
  );
}

/**
 * The sections as the user sees them: the daemon's copy with their edits laid over
 * it until the daemon's copy brings them. A repaint between two edits never
 * puts back the text of a section whose edit has not been saved yet. A note
 * written again by the hub starts from its new text.
 */
function useSectionEdits(note: OpenNoteSnapshot, send: Send) {
  const version = note.summary?.generated_at_ms ?? null;
  const local = useRef({ version, edits: new Map<number, string>() });
  if (local.current.version !== version) local.current = { version, edits: new Map() };
  const edits = local.current.edits;
  const base = (note.summary?.sections ?? []).map(({ markdown, name }) => ({ markdown, name }));
  for (const [index, markdown] of edits) {
    if (base[index]?.markdown === markdown) edits.delete(index);
  }
  const merged = (): Section[] =>
    base.map((section, index) => ({ ...section, markdown: edits.get(index) ?? section.markdown }));
  const save = useDebouncedCommit<Section[]>((value) => {
    void send({ command: "note_edit", recording_id: note.recording_id, sections: value });
  });
  // An edit repaints the tab, so its section says «Guardando…» until the note carries it.
  const [, repaint] = useState(0);
  const change = (index: number, markdown: string) => {
    edits.set(index, markdown);
    save.schedule(merged());
    repaint((count) => count + 1);
  };
  return {
    sections: merged(),
    change,
    flush: save.flush,
    unsaved: (index: number) => edits.has(index),
  };
}

/** Before the note is written: what will come, in the shape it will have. */
function Unwritten({
  library,
  note,
}: {
  library?: NoteLibrarySnapshot | null;
  note: OpenNoteSnapshot;
}) {
  if (notRequested(note)) return <p className={styles.quiet}>{es.noteScreen.notRequested}</p>;
  const id = templateOf(note);
  const template = library?.items.find((item) => item.id === id)?.name ?? id;
  return (
    <div aria-busy="true" className={styles.unwritten}>
      <p className={styles.quiet}>{es.noteScreen.summaryLater(template)}</p>
      <Skeleton height={12} widths={["140px"]} />
      <Skeleton height={10} widths={["100%", "90%", "64%"]} />
    </div>
  );
}

/** The summary the hub wrote, by template section, editable in place. */
export function SummaryTab({
  library,
  note,
  rewriting,
  send,
  writing,
}: {
  library?: NoteLibrarySnapshot | null;
  note: OpenNoteSnapshot;
  rewriting: Rewriting;
  send: Send;
  writing?: NoteWriting;
}) {
  const summary = note.summary;
  const { sections, change, flush, unsaved } = useSectionEdits(note, send);
  if (!summary)
    return (
      <>
        <SummaryTools note={note} rewriting={rewriting} />
        <Unwritten library={library} note={note} />
      </>
    );
  return (
    <div className={styles.summary}>
      <SummaryTools note={note} rewriting={rewriting} />
      <Superseded note={note} send={send} />
      {summary.sections.map((section, index) => (
        <SummarySection
          key={`${section.name}-${index}`}
          onChange={(markdown) => change(index, markdown)}
          onDone={flush}
          saving={unsaved(index)}
          section={{ ...section, markdown: sections[index]?.markdown ?? section.markdown }}
          writing={writing}
        />
      ))}
      {(note.decisions.length > 0 || note.open_topics.length > 0) && (
        <div className={styles.twoColumns}>
          <List items={note.decisions} title={es.noteScreen.decisions} />
          <List items={note.open_topics} title={es.noteScreen.openTopics} />
        </div>
      )}
      <WhoSpoke note={note} />
    </div>
  );
}
