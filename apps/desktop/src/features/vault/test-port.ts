import { vi } from "vitest";
import type { VaultNote } from "../../bridge/generated/VaultNote";
import type { VaultCatalog } from "../../bridge/generated/VaultCatalog";
import type { VaultTaskPage } from "../../bridge/generated/VaultTaskPage";
import type { VaultPort } from "./api";
import fixture from "./__fixtures__/reading.json";
export function fixturePort() {
  return vi.fn<VaultPort>(async (call) => {
    const configuration = {
      configured: true,
      kind: "folder",
      path: "/invented/Garden",
      url: "",
      vault: "Garden",
      has_token: false,
      link_style: "markdown",
      maintenance_model: "",
      maintenance_enabled: false,
    };
    if (call.call !== "read") return { outcome: "config", configuration };
    if (call.request.operation === "catalog")
      return { outcome: "catalog", catalog: fixture.catalog as VaultCatalog };
    if (call.request.operation === "tasks") {
      const tasks = fixture.tasks as VaultTaskPage;
      return {
        outcome: "tasks",
        tasks: {
          ...tasks,
          tasks: tasks.tasks.filter(
            (t) => call.request.query === "" || t.who.includes(call.request.query),
          ),
        },
      };
    }
    if (call.request.operation === "backlinks" || call.request.operation === "search")
      return {
        outcome: "page",
        page: {
          entries: [
            {
              path: "Projects/garden-pilot.md",
              kind: "note",
              title: "Garden Pilot",
              type: "project",
              snippet: "Con Ada Vale",
            },
          ],
          next_page: null,
        },
      };
    const note = (fixture.documents as Record<string, VaultNote>)[call.request.path];
    if (note === undefined)
      return {
        outcome: "refused",
        code: "vault_note_missing",
        message: "La nota no existe en el vault.",
        numbers: {},
      };
    return { outcome: "note", note: { ...note, next_page: null } };
  });
}
