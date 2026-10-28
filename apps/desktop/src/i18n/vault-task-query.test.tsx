import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { CopyProvider } from "./CopyProvider";
import { VaultMarkdown } from "../features/vault/VaultMarkdown";
import type { VaultNote } from "../bridge/generated/VaultNote";

const note: VaultNote = {
  path: "example.md",
  type: "note",
  role: "",
  properties: {},
  schema_warnings: [],
  conformance: { conformant: true, issues: [] },
  dependency_tasks: [],
  frontmatter: {},
  body: "```tasks\nnot done\n```",
  outlinks: [],
  synthesized: false,
  raw: "",
  citations: [],
  tasks: [],
  dependencies: [],
  queries: [],
  history: [],
  history_next_page: null,
  next_page: null,
};

afterEach(cleanup);

it("changes a missing task query's message with the live language", () => {
  const view = render(
    <CopyProvider choice="es">
      <VaultMarkdown note={note} notes={[]} />
    </CopyProvider>,
  );
  expect(screen.getByRole("alert").textContent).toBe("No se pudo leer esta consulta de tareas.");
  view.rerender(
    <CopyProvider choice="en">
      <VaultMarkdown note={note} notes={[]} />
    </CopyProvider>,
  );
  expect(screen.getByRole("alert").textContent).toBe("Couldn’t read this task query.");
});
