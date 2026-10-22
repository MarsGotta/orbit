import { useEffect, useState } from "react";
import type { AppCommand } from "../../../bridge/generated";
import type { DaemonConnection } from "../../../bridge/generated/DaemonConnection";
import type { NoteLibrarySnapshot } from "../../../bridge/generated/NoteLibrarySnapshot";
import type { NoteTemplateDetailSnapshot } from "../../../bridge/generated/NoteTemplateDetailSnapshot";
import { Card } from "../../../design/Card";
import { SectionLabel } from "../../../design/SectionLabel";
import { Toast } from "../../../design/Toast";
import { es } from "../../../i18n/es";
import { LibraryStatus } from "./LibraryStatus";
import { type Restoring, restoreNotice } from "./restore-notice";
import {
  type RemovedTemplate,
  draftOf,
  newTemplateId,
  restoreCommand,
  saveCommand,
} from "./template-draft";
import { TemplateEditor } from "./TemplateEditor";
import { TemplateList } from "./TemplateList";
import styles from "./Templates.module.css";
import { HUB_CONFIRMATION_MS } from "../../../state/ceilings";

interface Editing {
  /** One editing session: a new one starts only when the user picks another. */
  key: number;
  template: NoteTemplateDetailSnapshot;
}

/** A template of the user's, new, with one section to start from. */
function blank(): NoteTemplateDetailSnapshot {
  return {
    id: newTemplateId(),
    name: es.library.newTemplateName,
    context: "",
    sections: [{ name: es.library.newSectionName, instructions: "" }],
    builtin: false,
    forked_from: null,
    pending: true,
  };
}

/** One delete the user can still undo; `key` restarts the ten seconds. */
interface Undoable {
  key: number;
  removed: RemovedTemplate;
}

/** A promise it returns settles once the daemon answered the order. */
type Send = (command: AppCommand) => unknown;

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

/** The delete the user can still undo, and the undo they pressed. */
function useDeleteUndo(send: Send, error: string | null, daemon: DaemonConnection) {
  const [deleted, setDeleted] = useState<Undoable | null>(null);
  const [restoring, setRestoring] = useState<Restoring | null>(null);
  // A daemon that is not ready while an undo waits may have restarted, and
  // a restart loses the answers it kept in memory.
  useEffect(() => {
    if (daemon !== "ready")
      setRestoring((current) =>
        current === null || current.interrupted ? current : { ...current, interrupted: true },
      );
  }, [daemon]);
  return {
    deleted,
    restoring,
    onClose: () => setDeleted(null),
    onUndo: (removed: RemovedTemplate) => {
      const requestId = newTemplateId();
      setRestoring({
        removed,
        requestId,
        settled: false,
        errorBefore: error,
        interrupted: daemon !== "ready",
      });
      // Nothing settles an undo the hub never answers: past this, it is unknown.
      setTimeout(
        () =>
          setRestoring((current) =>
            current?.requestId === requestId ? { ...current, expired: true } : current,
          ),
        HUB_CONFIRMATION_MS,
      );
      void Promise.resolve(send(restoreCommand(removed, requestId))).then(() =>
        setRestoring((current) =>
          current?.removed === removed ? { ...current, settled: true } : current,
        ),
      );
    },
    deletedNow: (removed: RemovedTemplate) => {
      setRestoring(null);
      setDeleted((current) => ({ key: (current?.key ?? 0) + 1, removed }));
    },
  };
}

interface NoteTemplatesProps {
  /** The connection with the daemon, which a restart takes through
   *  `connecting` or `lost`. */
  daemon?: DaemonConnection;
  /** The refusal of Settings' last order, as the view shows it. */
  error?: string | null;
  library: NoteLibrarySnapshot | null | undefined;
  send: Send;
}

/**
 * Ajustes → Notas → Plantillas: Orbit's templates first, marked as
 * such, then the user's own; create, edit, reorder and delete, saved as they type.
 */
export function NoteTemplates({
  daemon = "ready",
  error = null,
  library,
  send,
}: NoteTemplatesProps) {
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
  return (
    <section aria-label={es.settings.notes} className={styles.page}>
      <SectionLabel>{es.settings.notes}</SectionLabel>
      <Card>
        <h2 className={styles.title}>{es.library.templatesTitle}</h2>
        <p className={styles.hint}>{es.library.templatesIntro}</p>
        <LibraryStatus library={library} />
        <DeleteNotices {...notices} daemon={daemon} error={error} library={library} />
        {library != null && library.templates.state === "available" && (
          <div className={styles.columns}>
            <TemplateList library={library} onCreate={create} onOpen={open} selected={selected} />
            {editing !== null && (
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
            )}
          </div>
        )}
      </Card>
    </section>
  );
}
