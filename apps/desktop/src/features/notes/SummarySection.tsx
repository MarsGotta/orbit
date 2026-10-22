import { useState } from "react";
import type { NoteSectionSnapshot } from "../../bridge/generated/NoteSectionSnapshot";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import { MarkdownView } from "./MarkdownView";
import { NoteEditor } from "./NoteEditor";
import styles from "./Note.module.css";

/**
 * One section of the summary: read as formatted text, edited in place with
 * the same editor as the private notes. Each change goes to `onChange`;
 * the tab saves them after a pause.
 */
export function SummarySection({
  onChange,
  onDone,
  section,
}: {
  onChange: (markdown: string) => void;
  onDone: () => void;
  section: NoteSectionSnapshot;
}) {
  const [editing, setEditing] = useState(false);
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
        <NoteEditor
          autoFocus
          label={es.noteScreen.sectionLabel(section.name)}
          markdown={section.markdown}
          onChange={onChange}
          placeholder=""
        />
      ) : (
        <MarkdownView source={section.markdown} />
      )}
    </section>
  );
}
