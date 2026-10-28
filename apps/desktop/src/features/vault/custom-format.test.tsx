import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import type { VaultPort } from "./api";
import { fixturePort } from "./test-port";
import VaultView from "./VaultView";

afterEach(() => {
  cleanup();
  window.location.hash = "";
});

function customPort(): VaultPort {
  const fixture = fixturePort();
  return async (call) => {
    const result = await fixture(call);
    if (result.outcome === "catalog")
      return {
        ...result,
        catalog: {
          ...result.catalog,
          schema: {
            ...result.catalog.schema,
            fields: { ...result.catalog.schema.fields, start: "kickoff" },
          },
          notes: result.catalog.notes.map((note) => ({
            ...note,
            type:
              note.type === "person" ? "contact" : note.type === "project" ? "venture" : note.type,
            role: note.type === "person" ? "person" : note.type === "project" ? "project" : "note",
          })),
        },
      };
    if (result.outcome === "note") {
      const person = result.note.path.includes("People/");
      return {
        ...result,
        note: {
          ...result.note,
          type: person ? "contact" : "venture",
          role: person ? "person" : "project",
          properties: {
            start: "2026-11-01",
            stage: "active",
            status: "stable",
            description: "Custom description",
          },
          frontmatter: {
            ...result.note.frontmatter,
            type: person ? "contact" : "venture",
            kickoff: "2026-11-01",
          },
        },
      };
    }
    return result;
  };
}

it("a custom person fiche lists projects linking to it by role", async () => {
  window.location.hash = "#/vault/n/People%2Fada-vale.md";
  render(<VaultView port={customPort()} />);
  expect(await screen.findByText("contact", { selector: "span" })).not.toBeNull();
  const projects = await screen.findByText("Proyectos", { selector: "dt" });
  expect(
    within(projects.parentElement!).getByRole("link", { name: "Garden Pilot" }),
  ).not.toBeNull();
});

it("a custom project gets its project page and kickoff milestone", async () => {
  window.location.hash = "#/vault";
  render(<VaultView port={customPort()} />);
  const projects = await screen.findByRole("heading", { name: "Proyectos" });
  expect(
    within(projects.parentElement!).getByRole("link", { name: /Garden Pilot/ }),
  ).not.toBeNull();
  window.location.hash = "#/vault/n/Projects%2Fgarden-pilot.md";
  fireEvent(window, new Event("hashchange"));
  expect(await screen.findByRole("img", { name: "Diagrama de dependencias" })).not.toBeNull();
  expect(await screen.findByText("2026-11-01", { selector: "dd" })).not.toBeNull();
});

it("tasks with custom person and project types keep their owners and project grouping", async () => {
  window.location.hash = "#/vault/tasks";
  render(<VaultView port={customPort()} />);
  expect(await screen.findByRole("heading", { name: /Ada Vale/ })).not.toBeNull();
  fireEvent.click(screen.getByRole("radio", { name: "Por proyecto" }));
  expect(await screen.findByRole("heading", { name: /Garden Pilot/ })).not.toBeNull();
});

it("shows unsupported Tasks query lines in the block while rendering the understood result", async () => {
  window.location.hash = "#/vault/n/People%2Fada-vale.md";
  const fixture = fixturePort();
  render(
    <VaultView
      port={async (call) => {
        const result = await fixture(call);
        if (result.outcome !== "note") return result;
        const query = result.note.queries[0];
        if (query === undefined) return result;
        const body = "```tasks\nnot done\nsort by due\n```\n";
        return {
          ...result,
          note: {
            ...result.note,
            body,
            raw: body,
            queries: [
              {
                ...query,
                lines: ["not done", "sort by due"],
                warnings: ["UNTRUSTED QUERY WARNING"],
                diagnostics: [
                  { code: "query_line_unsupported", field: "sort by due", numbers: {} },
                ],
              },
            ],
          },
        };
      }}
    />,
  );
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "Esta línea de la consulta no se aplica: sort by due",
  );
  expect(screen.getByRole("checkbox", { name: /Prepare the airport demo/ })).toHaveProperty(
    "disabled",
    true,
  );
});

it("shows a malformed schema notice on the home page without hiding notes", async () => {
  window.location.hash = "#/vault";
  const fixture = fixturePort();
  const warning =
    "SCHEMA.md: types.client.role debe ser un nombre de campo (línea 3). Se usan los valores por defecto.";
  render(
    <VaultView
      port={async (call) => {
        const result = await fixture(call);
        return result.outcome === "catalog"
          ? {
              ...result,
              catalog: {
                ...result.catalog,
                schema: {
                  ...result.catalog.schema,
                  warnings: ["UNTRUSTED SCHEMA WARNING"],
                  diagnostics: [
                    { code: "schema_field_name", field: "types.client.role", numbers: { line: 3 } },
                  ],
                },
              },
            }
          : result;
      }}
    />,
  );
  expect(await screen.findByRole("alert")).toHaveProperty("textContent", warning);
  expect(await screen.findByRole("heading", { name: "Proyectos" })).not.toBeNull();
});
