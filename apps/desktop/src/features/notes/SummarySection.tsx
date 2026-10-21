import { useState } from "react";
import type { NoteSectionSnapshot } from "../../bridge/generated/NoteSectionSnapshot";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import { MarkdownView } from "./MarkdownView";
import { NoteEditor } from "./NoteEditor";
import styles from "./NoteScreen.module.css";

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
    <section className={styles.section} data-edited={section.edited}>
      <header className={styles.sectionHead}>
        <h2>{section.name}</h2>
        <Button
          aria-label={editing ? es.noteScreen.done : es.noteScreen.editSection(section.name)}
          onClick={() => {
            if (editing) onDone();
            setEditing(!editing);
          }}
          size={26}
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
