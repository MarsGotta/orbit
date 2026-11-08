import { getCopy } from "../../../i18n";
import { useCopy } from "../../../i18n/CopyProvider";
import { useState } from "react";
import type { AppCommand } from "../../../bridge/generated";
import type { DaemonConnection } from "../../../bridge/generated/DaemonConnection";
import type { NoteLibrarySnapshot } from "../../../bridge/generated/NoteLibrarySnapshot";
import type { NoteTemplateDetailSnapshot } from "../../../bridge/generated/NoteTemplateDetailSnapshot";
import { Toast } from "../../../design/Toast";

import { Group, SkeletonRows } from "../layout";
import { LibraryStatus } from "./LibraryStatus";
import { type Restoring, restoreNotice } from "./restore-notice";
import { type RemovedTemplate, draftOf, newTemplateId, saveCommand } from "./template-draft";
import { TemplateEditor } from "./TemplateEditor";
import { TemplateList } from "./TemplateList";
import styles from "./Templates.module.css";
import { type Undoable, useDeleteUndo } from "./use-delete-undo";

interface Editing {
  /** One editing session: a new one starts only when the user picks another. */
  key: number;
  template: NoteTemplateDetailSnapshot;
}

/** A template of the user's, new, with one section to start from. */
function blank(): NoteTemplateDetailSnapshot {
  return {
    id: newTemplateId(),
    name: getCopy().library.newTemplateName,
    context: "",
    sections: [{ name: getCopy().library.newSectionName, instructions: "" }],
    builtin: false,
    forked_from: null,
    pending: true,
  };
}

/**
 * «Plantilla «X» borrada · N series vuelven a General · Deshacer»
 * for ten seconds, as the calendar's; after «Deshacer», where it stands.
 */
function DeleteNotices({
  daemon,
  deleted,
  error,
  library,
  onClose,
  onUndo,
  restoring,
}: {
  daemon: DaemonConnection;
  deleted: Undoable | null;
  error: string | null;
  library: NoteLibrarySnapshot | null | undefined;
  onClose: () => void;
  onUndo: (removed: RemovedTemplate) => void;
  restoring: Restoring | null;
}) {
  const es = useCopy();
  const notice = restoring === null ? null : restoreNotice(restoring, library, error, daemon);
  return (
    <>
      {deleted !== null && (
        <Toast
          embedded
          key={deleted.key}
          message={es.library.deleted(deleted.removed.name)}
          onClose={onClose}
          onUndo={() => onUndo(deleted.removed)}
        />
      )}
      {notice !== null && (
        <p className={notice.kind === "alert" ? styles.alert : styles.hint} role={notice.kind}>
          {notice.text}
        </p>
      )}
    </>
  );
}

interface NoteTemplatesProps {
  /** The connection with the daemon, which a restart takes through
   *  `connecting` or `lost`. */
  daemon?: DaemonConnection;
  /** The refusal of Settings' last order, as the view shows it. */
  error?: string | null;
  library: NoteLibrarySnapshot | null | undefined;
  send: (command: AppCommand) => unknown;
}

/**
 * Settings › Meetings › Plantillas: Orbit's templates first, marked as
 * such, then the user's own; create, edit, reorder and delete, saved as they type.
 */
export function NoteTemplates({
  daemon = "ready",
  error = null,
  library,
  send,
}: NoteTemplatesProps) {
  const es = useCopy();
  const [selected, setSelected] = useState<string | null>(null);
  const [editing, setEditing] = useState<Editing | null>(null);
  const notices = useDeleteUndo(send, error, daemon);
  const open = (template: NoteTemplateDetailSnapshot) => {
    setSelected(template.id);
    setEditing({ key: (editing?.key ?? 0) + 1, template });
  };
  const create = () => {
    const template = blank();
    send(saveCommand({ id: template.id, forkedFrom: null }, draftOf(template)));
    open(template);
  };
  const known = library != null && library.templates.state !== "unknown";
  return (
    <Group label={es.settings.templates}>
      <LibraryStatus library={library} />
      <DeleteNotices {...notices} daemon={daemon} error={error} library={library} />
      {!known && <SkeletonRows rows={3} />}
      {library != null && library.templates.state === "available" && (
        <div className={styles.columns}>
          <TemplateList library={library} onCreate={create} onOpen={open} selected={selected} />
          {editing !== null ? (
            <TemplateEditor
              key={editing.key}
              library={library}
              onCopied={setSelected}
              onDeleted={(removed) => {
                setSelected(null);
                setEditing(null);
                notices.deletedNow(removed);
              }}
              send={send}
              template={editing.template}
            />
          ) : (
            <p className={styles.choose}>{es.library.chooseTemplate}</p>
          )}
        </div>
      )}
    </Group>
  );
}
