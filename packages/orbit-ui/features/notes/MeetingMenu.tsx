import { useCopy } from "../../i18n/CopyProvider";
import { useCallback, useState } from "react";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { Popover } from "../../design/Popover";

import { sent, type Send } from "./send";
import styles from "./Note.module.css";

/**
 * «Más ⌄» in the note's header, with «Borrar reunión». The first press only
 * asks, in the same menu (`Popover`: a large change is confirmed as its last
 * step): what goes, where, and that it cannot be undone. Once the order is on
 * this Mac the note is left; it is gone from «Reuniones» at once. A meeting
 * still closing says it is deleted once it has closed.
 */
export function MeetingMenu({ note, send }: { note: OpenNoteSnapshot; send: Send }) {
  const es = useCopy();
  const [open, setOpen] = useState(false);
  const [asking, setAsking] = useState(false);
  const toggle = useCallback((next: boolean) => {
    setOpen(next);
    if (!next) setAsking(false);
  }, []);
  const closing = note.progress.upload.state !== "done";
  const remove = () => {
    void sent(send, { command: "meeting_delete", recording_id: note.recording_id }).then(
      () => {
        toggle(false);
        window.location.hash = "#/notes";
      },
      () => toggle(false),
    );
  };
  return (
    <Popover
      align="end"
      label={es.noteScreen.meetingMenuTitle}
      onOpenChange={toggle}
      open={open}
      trigger={
        <>
          {es.noteScreen.meetingMenu}
          <Icon name="chevron" size={14} />
        </>
      }
      triggerClassName={styles.howTrigger}
    >
      {asking ? (
        <div aria-label={es.noteScreen.deleteQuestion} className={styles.menu} role="group">
          <h2 className={styles.menuTitle}>{es.noteScreen.deleteQuestion}</h2>
          <p className={styles.menuWarning}>{es.noteScreen.deleteDetail}</p>
          {closing && <p className={styles.menuHelp}>{es.noteScreen.deleteWhenClosed}</p>}
          <div className={styles.menuActions}>
            <Button onClick={() => toggle(false)} size={32} variant="ghost">
              {es.controls.cancel}
            </Button>
            <Button onClick={remove} size={32} variant="dangerSoft">
              {es.noteScreen.deleteMeeting}
            </Button>
          </div>
        </div>
      ) : (
        <div className={styles.menu}>
          <h2 className={styles.menuTitle}>{es.noteScreen.meetingMenuTitle}</h2>
          <div className={styles.menuActions}>
            <Button onClick={() => setAsking(true)} size={32} variant="dangerSoft">
              {es.noteScreen.deleteMeeting}
            </Button>
          </div>
          {closing && <p className={styles.menuHelp}>{es.noteScreen.deleteWhenClosed}</p>}
        </div>
      )}
    </Popover>
  );
}
