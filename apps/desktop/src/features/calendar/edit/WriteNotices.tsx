import { Button } from "../../../design/Button";
import { Toast } from "../../../design/Toast";
import { es } from "../../../i18n/es";
import type { useCalendarWrites } from "./useCalendarWrites";
import styles from "./EventPanel.module.css";

type Writes = ReturnType<typeof useCalendarWrites>;

/**
 * What the calendar says about its writes (P9-22): a refusal or an unknown
 * outcome with its cause and numbers, a gesture Orbit did not try and why,
 * and «Evento borrado · Deshacer» for ten seconds after a delete.
 */
export function WriteNotices({ writes }: { writes: Writes }) {
  const { deleted, notice } = writes;
  return (
    <>
      {notice !== null && (
        <div
          className={styles.notice}
          data-kind={notice.kind}
          role={notice.kind === "info" ? "status" : "alert"}
        >
          <span>{notice.text}</span>
          <Button
            aria-label={es.calendar.write.dismiss}
            onClick={writes.dismissNotice}
            size={26}
            variant="ghost"
          >
            ×
          </Button>
        </div>
      )}
      {deleted !== null && (
        <Toast
          key={deleted.requestId}
          message={deleted.text}
          onClose={writes.dismissDeleted}
          onUndo={() => writes.undoDeleted(deleted.requestId)}
        />
      )}
    </>
  );
}
