import { describe, expect, it } from "vitest";
import type { NoteLibrarySnapshot } from "../../../bridge/generated/NoteLibrarySnapshot";
import { restoreNotice, type Restoring } from "./restore-notice";
import type { RemovedTemplate } from "./template-draft";

// «Deshacer» after deleting a template: on its way until the library shows it
// back; past its ceiling with nothing settled, what became of it is unknown.

const removed = {
  id: "tpl-retro",
  name: "Retrospectiva",
  series: null,
} as unknown as RemovedTemplate;
const restoring: Restoring = {
  removed,
  requestId: "restore-1",
  settled: true,
  errorBefore: null,
  interrupted: false,
};
const library = (items: { id: string; pending: boolean }[], answered = false) =>
  ({
    items,
    refusals: [],
    restores: answered ? [{ request_id: "restore-1" }] : [],
    assignments: [],
  }) as unknown as NoteLibrarySnapshot;

describe("an undo of a deleted template", () => {
  it("is on its way while nothing answered it", () => {
    expect(restoreNotice(restoring, library([]), null)).toEqual({
      kind: "status",
      text: "Recuperando «Retrospectiva»…",
    });
  });

  it("past its ceiling with no answer, is not said to be on its way for ever", () => {
    expect(restoreNotice({ ...restoring, expired: true }, library([]), null)).toEqual({
      kind: "status",
      text: "No se sabe si «Retrospectiva» se recuperó. Compruébalo en la lista.",
    });
  });

  it("past its ceiling still held by the hub, is not said to be on its way either", () => {
    const held = library([{ id: "tpl-retro", pending: true }], true);
    expect(restoreNotice({ ...restoring, expired: true }, held, null).text).toBe(
      "No se sabe si «Retrospectiva» se recuperó. Compruébalo en la lista.",
    );
  });

  it("once back, is back, ceiling or not", () => {
    const back = library([{ id: "tpl-retro", pending: false }], true);
    expect(restoreNotice({ ...restoring, expired: true }, back, null).text).toBe(
      "Plantilla «Retrospectiva» recuperada",
    );
  });
});
