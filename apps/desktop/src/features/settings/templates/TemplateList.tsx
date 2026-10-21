import type { NoteLibrarySnapshot } from "../../../bridge/generated/NoteLibrarySnapshot";
import type { NoteTemplateDetailSnapshot } from "../../../bridge/generated/NoteTemplateDetailSnapshot";
import { Button } from "../../../design/Button";
import { Tag } from "../../../design/Tag";
import { es } from "../../../i18n/es";
import styles from "./Templates.module.css";

/** Orbit's templates first, then the user's own; how many of theirs fit. */
export function TemplateList({
  library,
  onCreate,
  onOpen,
  selected,
}: {
  library: NoteLibrarySnapshot;
  onCreate: () => void;
  onOpen: (template: NoteTemplateDetailSnapshot) => void;
  selected: string | null;
}) {
  const ordered = [
    ...library.items.filter((template) => template.builtin),
    ...library.items.filter((template) => !template.builtin),
  ];
  const own = library.items.filter((template) => !template.builtin).length;
  const full = own >= library.bounds.max_custom_templates;
  return (
    <div className={styles.list}>
      <ul aria-label={es.library.templatesTitle}>
        {ordered.map((template) => (
          <li key={template.id}>
            <button
              aria-current={selected === template.id}
              className={styles.row}
              onClick={() => onOpen(template)}
              type="button"
            >
              <span>{template.name}</span>
              {template.builtin && <Tag>{es.library.fromOrbit}</Tag>}
              {template.pending && <small>{es.library.pending}</small>}
            </button>
          </li>
        ))}
      </ul>
      <Button disabled={full} onClick={onCreate} size={28} variant="outline">
        {es.library.newTemplate}
      </Button>
      {full && <p className={styles.hint}>{es.library.noRoom}</p>}
    </div>
  );
}
