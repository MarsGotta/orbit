import { useCallback, useEffect, useRef, useState } from "react";
import type { AppCommand } from "../../../bridge/generated";
import type { NoteTemplateDetailSnapshot } from "../../../bridge/generated/NoteTemplateDetailSnapshot";
import {
  type EditTarget,
  type TemplateDraft,
  draftOf,
  newTemplateId,
  saveCommand,
} from "./template-draft";

/** A template is saved one second after the user pauses, and at least
 * every five seconds while they keep typing. */
export const SAVE_AFTER_MS = 1_000;
export const SAVE_AT_LEAST_EVERY_MS = 5_000;

function useAutosave(commit: () => void) {
  const pause = useRef<number | null>(null);
  const ceiling = useRef<number | null>(null);
  const owed = useRef(false);
  const commitRef = useRef(commit);
  commitRef.current = commit;
  const flush = useCallback(() => {
    for (const timer of [pause, ceiling]) {
      if (timer.current !== null) window.clearTimeout(timer.current);
      timer.current = null;
    }
    if (owed.current) {
      owed.current = false;
      commitRef.current();
    }
  }, []);
  const schedule = useCallback(() => {
    owed.current = true;
    if (pause.current !== null) window.clearTimeout(pause.current);
    pause.current = window.setTimeout(flush, SAVE_AFTER_MS);
    ceiling.current ??= window.setTimeout(flush, SAVE_AT_LEAST_EVERY_MS);
  }, [flush]);
  const discard = useCallback(() => {
    owed.current = false;
    flush();
  }, [flush]);
  useEffect(() => flush, [flush]);
  return { schedule, flush, discard };
}

/**
 * The template being edited: the user's text stays as they type it (an echo of
 * their own save never rewrites it), and the first change of one of Orbit's
 * becomes a copy of their own with a new id.
 */
export function useTemplateEditor(
  template: NoteTemplateDetailSnapshot,
  send: (command: AppCommand) => void,
  onCopied: (id: string) => void,
) {
  const [draft, setDraft] = useState<TemplateDraft>(() => draftOf(template));
  const target = useRef<EditTarget>({ id: template.id, forkedFrom: null });
  const latest = useRef(draft);
  const save = useAutosave(() => send(saveCommand(target.current, latest.current)));
  const change = (next: TemplateDraft) => {
    if (template.builtin && target.current.id === template.id) {
      target.current = { id: newTemplateId(), forkedFrom: template.id };
      onCopied(target.current.id);
    }
    latest.current = next;
    setDraft(next);
    save.schedule();
  };
  return {
    draft,
    change,
    discard: save.discard,
    flush: save.flush,
    targetId: () => target.current.id,
  };
}
