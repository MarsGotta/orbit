/** Invented reading data from the owner's supplied example vault. No real connector. */
import type { VaultCall } from "../bridge/generated/VaultCall";
import type { VaultOutcome } from "../bridge/generated/VaultOutcome";
import type { VaultNote } from "../bridge/generated/VaultNote";
import type { VaultCatalog } from "../bridge/generated/VaultCatalog";
import type { VaultTaskPage } from "../bridge/generated/VaultTaskPage";
import fixture from "../features/vault/__fixtures__/reading.json";

export function vaultAnswer(call: VaultCall, empty: boolean): VaultOutcome {
  const configuration = {
    configured: !empty,
    kind: "folder",
    path: "/srv-example/notes/Nubaria",
    url: "",
    vault: "Nubaria",
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
          (task) =>
            (call.request.query === "" || task.who.includes(call.request.query)) &&
            (call.request.state !== "open" || !task.done),
        ),
      },
    };
  }
  if (call.request.operation === "backlinks" || call.request.operation === "search") {
    const entries = fixture.catalog.notes.filter((note) =>
      call.request.operation === "backlinks"
        ? note.links.includes(call.request.path)
        : note.title.toLocaleLowerCase().includes(call.request.query.toLocaleLowerCase()),
    );
    return { outcome: "page", page: { entries, next_page: null } };
  }
  const note = (fixture.documents as Record<string, VaultNote>)[call.request.path];
  return note === undefined
    ? {
        outcome: "refused",
        code: "vault_note_missing",
        message: "La nota no existe en el vault de ejemplo.",
        numbers: {},
      }
    : { outcome: "note", note };
}
