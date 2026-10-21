import type { NoteLibrarySnapshot } from "../../../bridge/generated/NoteLibrarySnapshot";
import { es } from "../../../i18n/es";
import { refusalText } from "../../../app/library-refusal";
import styles from "./Templates.module.css";

/**
 * What the template library has to say: a read that failed, changes
 * waiting for the hub, and what the hub refused, as what to do. Not read yet
 * is never «none» and says nothing.
 */
export function LibraryStatus({ library }: { library: NoteLibrarySnapshot | null | undefined }) {
  if (library == null) return null;
  const lines: string[] = [];
  if (library.read === "failed" || library.templates.state === "failed")
    lines.push(es.library.templatesFailed);
  else if (library.templates.state === "needs_hub") lines.push(es.library.needsHub);
  const known = new Set(library.items.map((template) => template.id));
  // Refusals of a template that is not in the list (a new one the hub did
  // not take) are said here; the others beside their template.
  const orphans = library.refusals.filter(
    (refusal) => refusal.target_kind !== "template" || !known.has(refusal.target_id),
  );
  return (
    <>
      {lines.map((line, index) => (
        <p className={styles.hint} key={index} role="status">
          {line}
        </p>
      ))}
      {orphans.map((refusal, index) => (
        <p className={styles.alert} key={`refusal-${index}`} role="alert">
          {refusalText(refusal)}
        </p>
      ))}
    </>
  );
}
