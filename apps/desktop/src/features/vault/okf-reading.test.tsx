import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import type { VaultPort } from "./api";
import { fixturePort } from "./test-port";
import VaultView from "./VaultView";

afterEach(() => {
  cleanup();
  window.location.hash = "";
});

function okfPort(): VaultPort {
  const fixture = fixturePort();
  return async (call) => {
    const result = await fixture(call);
    if (result.outcome === "catalog")
      return {
        ...result,
        catalog: {
          ...result.catalog,
          schema: { ...result.catalog.schema, fields: { stage: "phase", start: "kickoff" } },
          recent: [
            {
              date: "2026-10-28",
              time: "",
              action: "",
              title: "",
              path: "",
              by: "",
              detail: "",
              text: "* Checked the garden sources.",
            },
          ],
        },
      };
    if (result.outcome !== "note") return result;
    return {
      ...result,
      note: {
        ...result.note,
        properties: { ...result.note.properties, status: "deprecated", stage: "Pilot ready" },
        frontmatter: { ...result.note.frontmatter, status: "deprecated", phase: "Pilot ready" },
        history: [
          {
            date: "2026-10-28",
            time: "",
            action: "",
            title: "",
            path: result.note.path,
            by: "",
            detail: "",
            text: "* Checked this note's sources.",
          },
        ],
        conformance: {
          conformant: false,
          issues: [
            {
              field: "generated.by",
              code: "invalid_generated",
              message: "generated.by: autor no válido",
            },
          ],
        },
      },
    };
  };
}

it("shows project stage separately from the note lifecycle and keeps conformance invisible", async () => {
  window.location.hash = "#/vault/n/Projects%2Fgarden-pilot.md";
  render(<VaultView port={okfPort()} />);
  const lifecycle = await screen.findByText("Retirada");
  expect(lifecycle.closest("aside")?.getAttribute("aria-label")).toBe("Sobre esta nota");
  const stage = await screen.findByText("Estado del proyecto", { selector: "dt" });
  expect(within(stage.parentElement!).getByText("Pilot ready")).not.toBeNull();
  expect(screen.queryByText(/autor no válido/)).toBeNull();
});

it("shows free prose log entries as plain text in recent activity", async () => {
  window.location.hash = "#/vault";
  render(<VaultView port={okfPort()} />);
  expect(await screen.findByText("* Checked the garden sources.")).not.toBeNull();
});

it("shows free prose log entries in a note's history", async () => {
  window.location.hash = "#/vault/n/Projects%2Fgarden-pilot.md";
  render(<VaultView port={okfPort()} />);
  expect(await screen.findByText("* Checked this note's sources.")).not.toBeNull();
});
