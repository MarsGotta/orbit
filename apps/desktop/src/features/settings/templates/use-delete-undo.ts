import { useEffect, useRef, useState } from "react";
import type { AppCommand } from "../../../bridge/generated";
import type { DaemonConnection } from "../../../bridge/generated/DaemonConnection";
import { HUB_CONFIRMATION_MS } from "../../../state/ceilings";
import { useTemplateUndoKeyboard, useWindowActions } from "../../../state/window-actions";
import type { Restoring } from "./restore-notice";
import { type RemovedTemplate, newTemplateId, restoreCommand } from "./template-draft";

/** One delete the user can still undo; `key` restarts the ten seconds. */
export interface Undoable {
  key: number;
  removed: RemovedTemplate;
}

/** A promise it returns settles once the daemon answered the order. */
type Send = (command: AppCommand) => unknown;

/** The delete the user can still undo, and the undo they pressed. */
export function useDeleteUndo(send: Send, error: string | null, daemon: DaemonConnection) {
  useTemplateUndoKeyboard();
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
  const onUndo = (removed: RemovedTemplate) => {
    useWindowActions.setState({ undoTemplate: null });
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
  };
  const latestUndo = useRef(onUndo);
  latestUndo.current = onUndo;
  return {
    deleted,
    restoring,
    onClose: () => setDeleted(null),
    onUndo,
    deletedNow: (removed: RemovedTemplate) => {
      useWindowActions.setState({ undoTemplate: () => latestUndo.current(removed) });
      setRestoring(null);
      setDeleted((current) => ({ key: (current?.key ?? 0) + 1, removed }));
    },
  };
}
