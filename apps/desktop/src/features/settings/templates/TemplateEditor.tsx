import type { AppCommand } from "../../../bridge/generated";
import type { NoteLibrarySnapshot } from "../../../bridge/generated/NoteLibrarySnapshot";
import type { NoteTemplateDetailSnapshot } from "../../../bridge/generated/NoteTemplateDetailSnapshot";
import { Button } from "../../../design/Button";
import { es } from "../../../i18n/es";
import { refusalText } from "../../../app/library-refusal";
import { type RemovedTemplate, type TemplateDraft, removedTemplate } from "./template-draft";
import { TemplateSections } from "./TemplateSections";
import { useTemplateEditor } from "./use-template-editor";
import styles from "./Templates.module.css";

function EditorNotices({
  library,
  own,
  targetId,
  template,
}: {
  library: NoteLibrarySnapshot;
  own: boolean;
  targetId: string;
  template: NoteTemplateDetailSnapshot;
}) {
  const shown = library.items.find((item) => item.id === targetId);
  // A copy of one of Orbit's names it before the hub has answered.
  const origin =
    shown?.forked_from ??
    (template.builtin ? { id: template.id, name: template.name } : template.forked_from);
  const refusals = library.refusals.filter(
    (refusal) => refusal.target_kind === "template" && refusal.target_id === targetId,
  );
  return (
    <>
      {template.builtin && !own && <p className={styles.hint}>{es.library.builtinHint}</p>}
      {origin != null && own && <p className={styles.hint}>{es.library.copyOf(origin.name)}</p>}
      {shown?.pending === true && <p className={styles.hint}>{es.library.pending}</p>}
      {refusals.map((refusal, index) => (
        <p className={styles.alert} key={index} role="alert">
          {refusalText(refusal)}
        </p>
      ))}
    </>
  );
}

function TemplateFields({
  bounds,
  change,
  draft,
  flush,
}: {
  bounds: NoteLibrarySnapshot["bounds"];
  change: (draft: TemplateDraft) => void;
  draft: TemplateDraft;
  flush: () => void;
}) {
  return (
    <>
      <label className={styles.field}>
        <span>{es.library.name}</span>
        <input
          maxLength={bounds.max_name_chars}
          onBlur={flush}
          onChange={(event) => change({ ...draft, name: event.target.value })}
          value={draft.name}
        />
      </label>
      <label className={styles.field}>
        <span>{es.library.context}</span>
        <textarea
          maxLength={bounds.max_context_chars}
          onBlur={flush}
          onChange={(event) => change({ ...draft, context: event.target.value })}
          placeholder={es.library.contextHint}
          rows={3}
          value={draft.context}
        />
        <small>{es.library.characters(draft.context.length, bounds.max_context_chars)}</small>
      </label>
    </>
  );
}

/**
 * One template: name, context and sections, saved a second after the user
 * pauses. One of Orbit's is never changed: the first change saves a copy of
 * the user's own that says where it comes from. Deleting asks nothing.
 */
export function TemplateEditor({
  library,
  onCopied,
  onDeleted,
  send,
  template,
}: {
  library: NoteLibrarySnapshot;
  onCopied: (id: string) => void;
  /** What «Deshacer» brings back. */
  onDeleted: (removed: RemovedTemplate) => void;
  send: (command: AppCommand) => void;
  template: NoteTemplateDetailSnapshot;
}) {
  const { draft, change, discard, flush, targetId } = useTemplateEditor(template, send, onCopied);
  const bounds = library.bounds;
  const own = !template.builtin || targetId() !== template.id;
  return (
    <section aria-label={draft.name} className={styles.editor}>
      <EditorNotices library={library} own={own} targetId={targetId()} template={template} />
      <TemplateFields bounds={bounds} change={change} draft={draft} flush={flush} />
      <TemplateSections
        bounds={bounds}
        onChange={(sections) => change({ ...draft, sections })}
        sections={draft.sections}
      />
      {own && (
        <Button
          onClick={() => {
            // Nothing typed before the delete is saved after it.
            discard();
            const id = targetId();
            const removed = removedTemplate(library, id, draft);
            send({
              command: "note_library",
              request: { action: "delete_template", template_id: id },
            });
            onDeleted(removed);
          }}
          size={28}
          variant="ghost"
        >
          {es.library.deleteTemplate}
        </Button>
      )}
    </section>
  );
}
