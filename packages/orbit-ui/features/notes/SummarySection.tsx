import { useCopy } from "../../i18n/CopyProvider";
import { useState } from "react";
import type { NoteSectionSnapshot } from "../../bridge/generated/NoteSectionSnapshot";
import { Button } from "../../design/Button";
import { Spinner } from "../../design/Spinner";

import { MarkdownEditor } from "../editor/MarkdownEditor";
import { MarkdownView } from "./MarkdownView";
import type { NoteWriting } from "./note-writing";
import styles from "./Note.module.css";

/**
 * «Guardando…» until the note carries the edit; then «Guardado en este Mac»
 * while the hub has not taken it, and «✓ Guardado» once it has.
 */
function SectionSaving({ onMac, saving }: { onMac: boolean; saving: boolean }) {
  const es = useCopy();
  if (saving) {
    return (
      <>
        <Spinner size={12} />
        {es.editor.saving}
      </>
    );
  }
  return <>{onMac ? es.editor.savedMac : es.editor.savedHub}</>;
}

/**
 * One section of the summary: read as formatted text, edited in place in
 * the notes' editor. Each change goes to `onChange`; the tab saves them a
 * second after a pause, and at once when the text loses the focus
 * (`onDone`). Its status says where the edit is (`SectionSaving`).
 */
export function SummarySection({
  onChange,
  onDone,
  onMac,
  saving,
  section,
  writing,
}: {
  onChange: (markdown: string) => void;
  onDone: () => void;
  /** An edit of the sections this Mac keeps and the hub has not taken yet. */
  onMac: boolean;
  /** An edit of this section the note does not carry yet. */
  saving: boolean;
  section: NoteSectionSnapshot;
  writing?: NoteWriting;
}) {
  const es = useCopy();
  const [editing, setEditing] = useState(false);
  const [edited, setEdited] = useState(false);
  return (
    <section className={styles.section} data-edited={section.edited} data-editing={editing}>
      <header className={styles.sectionHead}>
        <h3>{section.name}</h3>
        <Button
          aria-label={editing ? es.noteScreen.done : es.noteScreen.editSection(section.name)}
          className={styles.sectionEdit}
          onClick={() => {
            if (editing) onDone();
            setEditing(!editing);
          }}
          size={28}
          variant="ghost"
        >
          {editing ? es.noteScreen.done : es.noteScreen.edit}
        </Button>
      </header>
      {editing ? (
        <MarkdownEditor
          assistant={writing?.assistant}
          autoFocus
          label={es.noteScreen.sectionLabel(section.name)}
          notes={writing?.notes}
          onBlur={onDone}
          onChange={(markdown) => {
            setEdited(true);
            onChange(markdown);
          }}
          people={writing?.people}
          status={(edited || onMac) && <SectionSaving onMac={onMac} saving={saving} />}
          value={section.markdown}
        />
      ) : (
        <MarkdownView source={section.markdown} />
      )}
    </section>
  );
}
