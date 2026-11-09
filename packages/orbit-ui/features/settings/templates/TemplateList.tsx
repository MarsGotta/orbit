import { useCopy } from "../../../i18n/CopyProvider";
import type { NoteLibrarySnapshot } from "../../../bridge/generated/NoteLibrarySnapshot";
import type { NoteTemplateDetailSnapshot } from "../../../bridge/generated/NoteTemplateDetailSnapshot";
import { Button } from "../../../design/Button";
import { Icon } from "../../../design/Icon";

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
  const es = useCopy();
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
              <span className={styles.rowName}>{template.name}</span>
              {template.pending ? (
                <small>{es.library.pending}</small>
              ) : (
                template.builtin && <small>{es.library.fromOrbit}</small>
              )}
            </button>
          </li>
        ))}
      </ul>
      <Button
        className={styles.add}
        disabledReason={full ? es.library.noRoom : undefined}
        onClick={onCreate}
        size={32}
        variant="ghost"
      >
        <Icon name="plus" size={14} />
        {es.library.newTemplate}
      </Button>
    </div>
  );
}
