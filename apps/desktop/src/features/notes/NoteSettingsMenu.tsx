import { useCallback, useRef, useState } from "react";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";
import { type Feedback, useFeedback } from "../../design/feedback";
import { Icon } from "../../design/Icon";
import { Popover } from "../../design/Popover";
import { es } from "../../i18n/es";
import { languageName } from "./notes-format";
import {
  type Choice,
  current,
  howLabel,
  orders,
  refusals,
  rewrites,
  templateName,
} from "./note-settings";
import { Fields } from "./NoteSettingsFields";
import { sent, type Send } from "./send";
import styles from "./Note.module.css";

/** «Cerrar», and the one action the choice needs: «Reescribir la nota» or «Guardar». */
function Footer({
  blocked,
  changed,
  rewrite,
  feedback,
  onApply,
  onClose,
}: {
  blocked?: string;
  changed: boolean;
  rewrite: boolean;
  feedback: Feedback;
  onApply: () => void;
  onClose: () => void;
}) {
  return (
    <div className={styles.menuActions}>
      <Button onClick={onClose} size={32} variant="ghost">
        {es.noteScreen.close}
      </Button>
      {rewrite && (
        <Button disabledReason={blocked} onClick={onApply} size={32} variant="accent">
          {es.noteScreen.rewriteNote}
        </Button>
      )}
      {!rewrite && changed && (
        <Button feedback={feedback} onClick={onApply} size={32} variant="accent">
          {es.noteScreen.save}
        </Button>
      )}
    </div>
  );
}

/** The menu's state: open, the choice being made, and what applying it sends. */
function useNoteSettings(
  note: OpenNoteSnapshot,
  library: NoteLibrarySnapshot | null | undefined,
  send: Send,
  onRewrite: (templateName: string, work: Promise<unknown>) => void,
) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<Choice>(() => current(note));
  const { feedback, track } = useFeedback();
  const latest = useRef(note);
  latest.current = note;
  // Stable: the popover focuses its first field each time this changes.
  const toggle = useCallback((next: boolean) => {
    if (next) setDraft(current(latest.current));
    setOpen(next);
  }, []);
  const from = current(note);
  const rewrite = rewrites(from, draft);
  const changed = rewrite || draft.series !== from.series;
  const apply = () => {
    const work = Promise.all(orders(note, from, draft).map((command) => sent(send, command)));
    if (rewrite) {
      onRewrite(templateName(library, draft.template, note), work);
      setOpen(false);
    } else track(work.then(() => setOpen(false)));
  };
  const language =
    draft.language !== null && draft.language !== from.language
      ? languageName(draft.language)
      : null;
  return { open, toggle, draft, setDraft, feedback, rewrite, changed, apply, language };
}

/**
 * «Plantilla: … · Español ⌄»: how Orbit writes this note. Nothing changes on
 * a mere choice (proposals 8 and 9): writing the note again waits for
 * «Reescribir la nota», which says what it does first, whether the template,
 * the language or the dictionaries changed; the series takes the template
 * and the dictionaries together.
 */
export function NoteSettingsMenu({
  blocked,
  library,
  note,
  onRewrite,
  send,
}: {
  /** Why the note cannot be written again now (while recording). */
  blocked?: string;
  library?: NoteLibrarySnapshot | null;
  note: OpenNoteSnapshot;
  onRewrite: (templateName: string, work: Promise<unknown>) => void;
  send: Send;
}) {
  const menu = useNoteSettings(note, library, send, onRewrite);
  return (
    <Popover
      label={es.noteScreen.howTitle}
      onOpenChange={menu.toggle}
      open={menu.open}
      trigger={
        <>
          {howLabel(note, library)}
          <Icon name="chevron" size={14} />
        </>
      }
      triggerClassName={styles.howTrigger}
    >
      <div className={styles.menu}>
        <h2 className={styles.menuTitle}>{es.noteScreen.howTitle}</h2>
        <Fields
          draft={menu.draft}
          library={library}
          note={note}
          set={(change) => menu.setDraft({ ...menu.draft, ...change })}
        />
        {refusals(note, library).map((text, index) => (
          <p className={styles.menuRefusal} key={index} role="alert">
            {text}
          </p>
        ))}
        {menu.rewrite && (
          <p className={styles.menuWarning}>{es.noteScreen.rewriteWarning(menu.language)}</p>
        )}
        <Footer
          blocked={blocked}
          changed={menu.changed}
          feedback={menu.feedback}
          onApply={menu.apply}
          onClose={() => menu.toggle(false)}
          rewrite={menu.rewrite}
        />
      </div>
    </Popover>
  );
}
