import { useState } from "react";
import type { NoteSectionSnapshot } from "../../bridge/generated/NoteSectionSnapshot";
import { Button } from "../../design/Button";
import { Spinner } from "../../design/Spinner";
import { es } from "../../i18n/es";
import { MarkdownEditor } from "../editor/MarkdownEditor";
import { MarkdownView } from "./MarkdownView";
import type { NoteWriting } from "./note-writing";
import styles from "./Note.module.css";

/** «Guardando…» until the note carries the edit, then «✓ Guardado». */
function SectionSaving({ saving }: { saving: boolean }) {
  if (!saving) return <>{es.editor.savedHub}</>;
  return (
    <>
      <Spinner size={12} />
      {es.editor.saving}
    </>
  );
}

/**
 * One section of the summary: read as formatted text, edited in place in
 * the notes' editor. Each change goes to `onChange`; the tab saves them a
 * second after a pause, and at once when the text loses the focus
 * (`onDone`). «Guardando…» until the note carries the edit, then «✓ Guardado».
 */
export function SummarySection({
  onChange,
  onDone,
  saving,
  section,
  writing,
}: {
  onChange: (markdown: string) => void;
  onDone: () => void;
  /** An edit of this section the note does not carry yet. */
  saving: boolean;
  section: NoteSectionSnapshot;
  writing?: NoteWriting;
}) {
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
          status={edited && <SectionSaving saving={saving} />}
          value={section.markdown}
        />
      ) : (
        <MarkdownView source={section.markdown} />
      )}
    </section>
  );
}
