import { type CSSProperties, useRef } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { Section } from "../../bridge/generated/Section";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import { summaryMarkdown } from "./notes-format";
import { SummarySection } from "./SummarySection";
import { useDebouncedCommit } from "./use-debounced-commit";
import styles from "./NoteScreen.module.css";
import listStyles from "./NotesView.module.css";

type Send = (command: AppCommand) => void;

function VoiceShare({ note }: { note: OpenNoteSnapshot }) {
  if (note.speakers.length === 0) return null;
  return (
    <section className={listStyles.voiceShare}>
      <h2>{es.notes.voiceShare}</h2>
      {note.speakers.map((speaker) => (
        <div
          className={listStyles.shareRow}
          key={speaker.key}
          style={
            {
              "--speaker-color": speaker.color,
              "--speaker-share": `${speaker.share_pct}%`,
            } as CSSProperties
          }
        >
          <span>{speaker.name}</span>
          <span className={listStyles.shareTrack}>
            <span />
          </span>
          <strong>{speaker.share_pct}%</strong>
        </div>
      ))}
    </section>
  );
}

function List({ items, title }: { items: string[]; title: string }) {
  if (items.length === 0) return null;
  return (
    <section className={styles.listBlock}>
      <h2>{title}</h2>
      <ul>
        {items.map((item, index) => (
          <li key={index}>{item}</li>
        ))}
      </ul>
    </section>
  );
}

function Notices({ note, send }: { note: OpenNoteSnapshot; send: Send }) {
  return (
    <>
      {note.summary?.superseded_available === true && (
        <p className={styles.notice} data-tone="quiet">
          <span>{es.noteScreen.supersededAvailable}</span>
          <Button
            onClick={() =>
              send({ command: "note_restore_superseded", recording_id: note.recording_id })
            }
            size={26}
            variant="ghost"
          >
            {es.noteScreen.restoreSuperseded}
          </Button>
        </p>
      )}
    </>
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
  const save = useDebouncedCommit<Section[]>((value) =>
    send({ command: "note_edit", recording_id: note.recording_id, sections: value }),
  );
  const change = (index: number, markdown: string) => {
    edits.set(index, markdown);
    save.schedule(merged());
  };
  return { sections: merged(), change, flush: save.flush };
}

/** The summary the hub wrote, by template section, editable in place. */
export function SummaryTab({ note, send }: { note: OpenNoteSnapshot; send: Send }) {
  const summary = note.summary;
  const { sections, change, flush } = useSectionEdits(note, send);
  if (!summary) {
    return (
      <section className={styles.summary}>
        <p className={styles.emptyCopy}>{es.notes.summaryLater}</p>
        <VoiceShare note={note} />
      </section>
    );
  }
  return (
    <section className={styles.summary}>
      <div className={styles.toolbar}>
        <Button
          onClick={() => void navigator.clipboard?.writeText(summaryMarkdown(note))}
          size={26}
          variant="ghost"
        >
          {es.noteScreen.copySummary}
        </Button>
      </div>
      <Notices note={note} send={send} />
      {summary.sections.map((section, index) => (
        <SummarySection
          key={`${section.name}-${index}`}
          onChange={(markdown) => change(index, markdown)}
          onDone={flush}
          section={{ ...section, markdown: sections[index]?.markdown ?? section.markdown }}
        />
      ))}
      <List items={note.decisions} title={es.notes.decisions} />
      <List items={note.open_topics} title={es.notes.openTopics} />
      <VoiceShare note={note} />
    </section>
  );
}
