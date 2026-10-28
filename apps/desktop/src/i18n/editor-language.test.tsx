import { render, screen, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { CopyProvider } from "./CopyProvider";
import { MarkdownEditor } from "../features/editor/MarkdownEditor";
import { editorNamed, focusAt, select, typeText } from "../features/editor/test-editor";

it("updates the editor placeholder and toolbar while preserving the draft and selection", () => {
  const onChange = vi.fn();
  const content = (choice: "es" | "en") => (
    <CopyProvider choice={choice}>
      <MarkdownEditor label="Draft" value="" onChange={onChange} />
    </CopyProvider>
  );
  const view = render(content("es"));
  expect(screen.getByText("Escribe o pulsa / para insertar…")).toBeTruthy();
  view.rerender(content("en"));
  expect(screen.getByText("Write or press / to insert…")).toBeTruthy();
  const editor = editorNamed("Draft");
  focusAt(editor);
  typeText(editor, "A new plan");
  select(editor, 2, 5);
  view.rerender(content("es"));
  expect(editorNamed("Draft")).toBe(editor);
  expect(editor.state.doc.toString()).toBe("A new plan");
  expect(editor.state.selection.main.from).toBe(2);
  expect(editor.state.selection.main.to).toBe(5);
  expect(
    within(screen.getByRole("toolbar")).getByRole("button", { name: "Negrita · ⌘B" }),
  ).toBeTruthy();
});

it("rewords task widgets and an unaccepted suggestion without editing the document", () => {
  const content = (choice: "es" | "en") => (
    <CopyProvider choice={choice}>
      <MarkdownEditor
        label="Draft"
        value={"- [ ] Plan\n\nDetails"}
        suggestion={{ author: "Atlas", text: "Proposed" }}
      />
    </CopyProvider>
  );
  const view = render(content("es"));
  expect(screen.getByRole("checkbox", { name: "Marcar tarea" })).toBeTruthy();
  expect(screen.getByRole("group", { name: "Escrito por Atlas" })).toBeTruthy();
  view.rerender(content("en"));
  expect(screen.getByRole("checkbox", { name: "Toggle task" })).toBeTruthy();
  expect(screen.getByRole("group", { name: "Written by Atlas" })).toBeTruthy();
  expect(editorNamed("Draft").state.doc.toString()).toBe("- [ ] Plan\n\nDetails");
});
