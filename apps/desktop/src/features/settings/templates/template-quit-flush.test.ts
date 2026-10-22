import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { AppCommand } from "../../../bridge/generated";
import type { NoteTemplateDetailSnapshot } from "../../../bridge/generated/NoteTemplateDetailSnapshot";
import { flushBeforeQuit } from "../../../state/quit-flush";
import { useTemplateEditor } from "./use-template-editor";

// A template typed less than a second before Orbit quits is saved before it
// goes: the quit waits for the daemon to take it.

const template: NoteTemplateDetailSnapshot = {
  id: "entrevista",
  name: "Entrevista",
  context: "Una persona candidata.",
  sections: [{ name: "Perfil", instructions: "" }],
  builtin: false,
  forked_from: null,
  pending: false,
};

afterEach(cleanup);

it("a template typed just before quitting is handed to the daemon before Orbit goes", async () => {
  let accept: () => void = () => undefined;
  const send = vi.fn<(command: AppCommand) => Promise<void>>(
    () => new Promise<void>((resolve) => (accept = resolve)),
  );
  const editor = renderHook(() => useTemplateEditor(template, send, () => undefined));
  act(() =>
    editor.result.current.change({ ...editor.result.current.draft, name: "Entrevista técnica" }),
  );
  // Still inside the second the editor waits for a pause.
  expect(send).not.toHaveBeenCalled();
  let done = false;
  const quitting = flushBeforeQuit().then(() => {
    done = true;
  });
  await Promise.resolve();
  expect(send).toHaveBeenCalledTimes(1);
  expect(send.mock.calls[0]?.[0]).toMatchObject({
    command: "note_library",
    request: { action: "save_template", draft: { name: "Entrevista técnica" } },
  });
  expect(done).toBe(false);
  accept();
  await quitting;
  expect(done).toBe(true);
});
