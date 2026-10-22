import { useEffect, useRef, useState } from "react";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import type { Section } from "../../bridge/generated/Section";
import { useLocalDraft } from "../../state/local-drafts";
import type { Send } from "./send";
import { useDebouncedCommit } from "./use-debounced-commit";

/** The edits of a note's sections not taken yet, over the note the hub wrote (`version`). */
interface SectionsDraft {
  version: number | null;
  edits: Record<string, string>;
}

function asSectionsDraft(raw: unknown): SectionsDraft | null {
  if (typeof raw !== "object" || raw === null) return null;
  const { version, edits } = raw as Record<string, unknown>;
  if (version !== null && typeof version !== "number") return null;
  if (typeof edits !== "object" || edits === null) return null;
  const entries = Object.entries(edits as Record<string, unknown>);
  if (!entries.every(([index, text]) => /^\d+$/.test(index) && typeof text === "string"))
    return null;
  return { version, edits: Object.fromEntries(entries) as Record<string, string> };
}

/**
 * The sections' edits kept on this Mac as they are typed, so that ⌘Q, an
 * update or a fall before the save loses nothing; they come back only over
 * the same writing of the note, and are let go of once the daemon took them.
 */
function useKeptSections(note: OpenNoteSnapshot) {
  const version = note.summary?.generated_at_ms ?? null;
  const kept = useLocalDraft(`note.${note.recording_id}.summary`, null, asSectionsDraft);
  const own = kept.value?.version === version ? kept.value : null;
  return {
    version,
    restored:
      own === null
        ? []
        : Object.entries(own.edits).map(([at, text]) => [Number(at), text] as const),
    recovered: kept.recovered && own !== null,
    keep: (edits: Map<number, string>) =>
      kept.set(edits.size === 0 ? null : { version, edits: Object.fromEntries(edits) }),
    taken: (sent: string) =>
      kept.set((now) => (now !== null && JSON.stringify(now.edits) === sent ? null : now)),
  };
}

/**
 * The sections as the user sees them: the daemon's copy with their edits laid over
 * it until the daemon's copy brings them. A repaint between two edits never
 * puts back the text of a section whose edit has not been saved yet. A note
 * written again by the hub starts from its new text.
 */
export function useSectionEdits(note: OpenNoteSnapshot, send: Send) {
  const kept = useKeptSections(note);
  const { version } = kept;
  const local = useRef<{ version: number | null; edits: Map<number, string> } | null>(null);
  if (local.current?.version !== version)
    local.current = { version, edits: new Map(kept.restored) };
  const edits = local.current.edits;
  const base = (note.summary?.sections ?? []).map(({ markdown, name }) => ({ markdown, name }));
  for (const [index, markdown] of edits) {
    if (base[index]?.markdown === markdown) edits.delete(index);
  }
  const merged = (): Section[] =>
    base.map((section, index) => ({ ...section, markdown: edits.get(index) ?? section.markdown }));
  const save = useDebouncedCommit<Section[]>((value) => {
    const sent = JSON.stringify(Object.fromEntries(edits));
    return Promise.resolve(
      send({ command: "note_edit", recording_id: note.recording_id, sections: value }),
    ).then((accepted) => {
      if (accepted !== false) kept.taken(sent);
    });
  });
  // An edit repaints the tab, so its section says «Guardando…» until the note carries it.
  const [, repaint] = useState(0);
  const change = (index: number, markdown: string) => {
    edits.set(index, markdown);
    kept.keep(edits);
    save.schedule(merged());
    repaint((count) => count + 1);
  };
  // Edits brought back from an earlier run go to the daemon as if just typed.
  const restoredOnce = useRef(edits.size > 0);
  useEffect(() => {
    if (restoredOnce.current) save.schedule(merged());
    restoredOnce.current = false;
  });
  return {
    sections: merged(),
    change,
    flush: save.flush,
    recovered: kept.recovered,
    unsaved: (index: number) => edits.has(index),
  };
}
