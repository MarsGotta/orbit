import type { VaultPort } from "./api";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { CopyProvider } from "../../i18n/CopyProvider";
import VaultView from "./VaultView";
import { fixturePort } from "./test-port";
import { proposal, proposalPort } from "./test-distillation";

afterEach(() => {
  cleanup();
  window.location.hash = "";
});
function open(value = proposal()) {
  window.location.hash = "#/vault/distill/cedar-proposal";
  const port = proposalPort(value);
  render(<VaultView port={port} />);
  return port;
}

it("53 blocks every undecided contradiction, even on an excluded note", async () => {
  const port = open();
  const apply = await screen.findByRole("button", { name: "Decide la fecha para aplicar" });
  expect((apply as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByRole("heading", { name: "Hay que decidir" })).toBeTruthy();
  fireEvent.click(screen.getByRole("checkbox", { name: "Incluir Cedar Pilot" }));
  await waitFor(() =>
    expect(port).toHaveBeenCalledWith(
      expect.objectContaining({
        call: "review_distillation",
        review: expect.objectContaining({ changes: { project: { include: false } } }),
      }),
    ),
  );
  expect(
    (screen.getByRole("button", { name: "Decide la fecha para aplicar" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
});

it("53a and 53b show section diffs, speaker/time evidence, and save an edited draft", async () => {
  const port = open();
  fireEvent.click(await screen.findByRole("button", { name: "Cedar Pilot · 2 cambios" }));
  expect(screen.getByText("Old plan", { selector: "del" })).toBeTruthy();
  expect(screen.getByText("New plan", { selector: "ins" })).toBeTruthy();
  expect(screen.getAllByText("01:05 · Nora Vale").length).toBeGreaterThan(0);
  expect(screen.getByRole("heading", { name: "Lo que se dijo" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Editar antes de aplicar" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Plan" }), {
    target: { value: "Edited plan" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Listo" }));
  await waitFor(() =>
    expect(port).toHaveBeenCalledWith(
      expect.objectContaining({
        call: "review_distillation",
        review: expect.objectContaining({
          changes: {
            project: { properties: { kickoff: "2026-11-02" }, lines: { "0": "Edited plan" } },
          },
        }),
      }),
    ),
  );
  expect(await screen.findByText("Edited plan", { selector: "ins" })).toBeTruthy();
});

it.each([
  ["Usar 2026-11-02", "incoming"],
  ["Mantener 2026-10-26", "current"],
  ["Dejar las dos y marcarlo como pendiente", "both"],
])("53c persists %s", async (label, choice) => {
  const port = open();
  fireEvent.click(await screen.findByRole("radio", { name: label }));
  await waitFor(() =>
    expect(port).toHaveBeenCalledWith(
      expect.objectContaining({
        call: "review_distillation",
        review: { revision: 0, decisions: { date: choice } },
      }),
    ),
  );
  expect(
    ((await screen.findByRole("button", { name: "Aplicar 3 cambios" })) as HTMLButtonElement)
      .disabled,
  ).toBe(false);
});

it("53d counts only included slots; 53e locks actions; 53f offers undo after confirmed apply", async () => {
  const port = open();
  fireEvent.click(await screen.findByRole("radio", { name: "Usar 2026-11-02" }));
  await screen.findByRole("button", { name: "Aplicar 3 cambios" });
  fireEvent.click(screen.getByRole("checkbox", { name: "Incluir Eli Brook" }));
  const apply = await screen.findByRole("button", { name: "Aplicar 2 cambios" });
  let done!: (value: Awaited<ReturnType<typeof port>>) => void;
  port.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        done = resolve;
      }),
  );
  fireEvent.click(apply);
  expect(
    ((await screen.findByRole("button", { name: "Aplicando…" })) as HTMLButtonElement).disabled,
  ).toBe(true);
  done({ outcome: "distillation", proposal: { ...proposal(), status: "applied" } });
  expect(await screen.findByText(/Aplicados 3 cambios en 2 notas/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Deshacer" }));
  expect(await screen.findByText(/La reunión sigue guardada como fuente/)).toBeTruthy();
});

it("discard states that no note or source was written", async () => {
  const port = open();
  fireEvent.click(await screen.findByRole("button", { name: "Descartar" }));
  expect(
    await screen.findByText(/No se ha cambiado ninguna nota ni guardado ninguna fuente/),
  ).toBeTruthy();
  expect(port).toHaveBeenCalledWith({
    call: "distillation_action",
    id: "cedar-proposal",
    action: "discard",
  });
});

it("stale names changed notes and prepares again; failures preserve numeric causes", async () => {
  open({ ...proposal(), status: "stale", changed_paths: ["Projects/cedar-pilot.md"] });
  expect(await screen.findByText(/Cedar Pilot.*cambió/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Volver a preparar" }));
  expect(await screen.findByRole("status")).toBeTruthy();
});

it("51 lists pending proposals in the column and home card", async () => {
  render(<VaultView port={proposalPort()} />);
  expect(await screen.findByText("Para revisar")).toBeTruthy();
  expect(await screen.findByRole("link", { name: "Revisar los cambios" })).toBeTruthy();
  expect(screen.getAllByText("Cedar review").length).toBeGreaterThan(0);
});

it.each([
  ["es", "Revisada por ti"],
  ["en", "Reviewed by you"],
])("owner provenance is readable in %s", async (locale, review) => {
  const base = fixturePort();
  window.location.hash = "#/vault/n/Projects%2Fgarden-pilot.md";
  render(
    <CopyProvider choice={locale as "es" | "en"}>
      <VaultView
        port={async (call) => {
          const answer = await base(call);
          if (answer.outcome === "catalog")
            return {
              ...answer,
              catalog: {
                ...answer.catalog,
                schema: { ...answer.catalog.schema, owner: "cedar-owner" },
              },
            };
          if (answer.outcome === "note")
            answer.note = {
              ...answer.note,
              frontmatter: {
                ...answer.note.frontmatter,
                generated: { by: "orbit-hub/0.14.0", at: "2026-10-30T11:52:00Z" },
                verified: [{ by: "human:cedar-owner", at: "2026-10-30T12:00:00Z" }],
              },
            };
          return answer;
        }}
      />
    </CopyProvider>,
  );
  expect(await screen.findByText(new RegExp(review))).toBeTruthy();
  const panel = screen.getByRole("complementary", {
    name: locale === "es" ? "Sobre esta nota" : "About this note",
  });
  expect(within(panel).queryByText(/human:|orbit-hub\//)).toBeNull();
});

it.each(["queued", "preparing"] as const)(
  "shows %s without permitting incomplete apply",
  async (status) => {
    open({ ...proposal(), status, changes: [], conflicts: [] });
    expect(
      await screen.findByText(
        status === "queued" ? "En cola para preparar el destilado…" : "Preparando el destilado…",
      ),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Aplicar/ })).toBeNull();
  },
);

it("shows the preparation cause with its numeric ceiling and retries explicitly", async () => {
  const port = open({
    ...proposal(),
    status: "failed",
    changes: [],
    conflicts: [],
    error: { code: "incomplete_completion", completion_tokens: 4096, max_completion_tokens: 4096 },
  });
  expect(await screen.findByText(/alcanzó su límite de salida.*4096/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Volver a intentar" }));
  await waitFor(() =>
    expect(port).toHaveBeenCalledWith({
      call: "distillation_action",
      id: "cedar-proposal",
      action: "prepare",
    }),
  );
});

it("reconciles an apply whose receipt was lost without repeating the write", async () => {
  const base = proposalPort({ ...proposal(), conflicts: [] });
  let wrote = false;
  const port = vi.fn<VaultPort>(async (call) => {
    if (call.call === "distillation_action" && call.action === "apply") {
      wrote = true;
      throw new Error("synthetic lost receipt");
    }
    if (call.call === "distillation" && wrote)
      return { outcome: "distillation", proposal: { ...proposal(), status: "applied" } };
    return base(call);
  });
  window.location.hash = "#/vault/distill/cedar-proposal";
  render(<VaultView port={port} />);
  fireEvent.click(await screen.findByRole("button", { name: "Aplicar 3 cambios" }));
  expect(await screen.findByRole("button", { name: "Deshacer" })).toBeTruthy();
  expect(port.mock.calls.filter(([call]) => call.call === "distillation_action").length).toBe(1);
  expect(await screen.findByRole("alert")).toBeTruthy();
});

it("keeps a rejected edit in the editor while rereading the authoritative proposal", async () => {
  const base = proposalPort();
  const port: typeof base = vi.fn(async (call) =>
    call.call === "review_distillation"
      ? { outcome: "refused", code: "distillation_edit_invalid", message: "synthetic", numbers: {} }
      : base(call),
  );
  window.location.hash = "#/vault/distill/cedar-proposal";
  render(<VaultView port={port} />);
  fireEvent.click(await screen.findByRole("button", { name: "Cedar Pilot · 2 cambios" }));
  fireEvent.click(screen.getByRole("button", { name: "Editar antes de aplicar" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Plan" }), {
    target: { value: "Keep this draft" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Listo" }));
  expect(await screen.findByRole("alert")).toBeTruthy();
  expect((screen.getByRole("textbox", { name: "Plan" }) as HTMLTextAreaElement).value).toBe(
    "Keep this draft",
  );
});

it("reports every undo skip with the note name and a distinct cause", async () => {
  open({
    ...proposal(),
    status: "undone",
    skipped: [
      { path: "Projects/cedar-pilot.md", code: "content_changed" },
      { path: "People/eli-brook.md", code: "outcome_unknown" },
    ],
  });
  expect(
    await screen.findByText(/Cedar Pilot no se ha deshecho: cambió después de aplicar/),
  ).toBeTruthy();
  expect(
    screen.getByText(/Eli Brook no se ha deshecho: no se conoce el resultado de la escritura/),
  ).toBeTruthy();
});

it("shows today’s author, fallback owner and source names without protocol actors or source paths", async () => {
  const base = fixturePort();
  const generated = new Date(new Date().setUTCHours(11, 52, 0, 0)).toISOString();
  window.location.hash = "#/vault/n/Projects%2Fgarden-pilot.md";
  render(
    <VaultView
      owner="Owen Reed"
      port={async (call) => {
        const answer = await base(call);
        if (answer.outcome === "note")
          return {
            ...answer,
            note: {
              ...answer.note,
              frontmatter: {
                ...answer.note.frontmatter,
                generated: { by: "orbit-hub/0.14.0", at: generated },
                verified: [{ by: "human:owen-reed", at: generated }],
              },
              citations: [
                {
                  id: "meeting",
                  number: 1,
                  title: "Cedar review · 21 oct",
                  resource: "/Sources/cedar-review.md",
                },
              ],
            },
          };
        return answer;
      }}
    />,
  );
  expect(await screen.findByText("Orbit · hoy, 11:52")).toBeTruthy();
  expect(screen.getByText(/Revisada por ti/)).toBeTruthy();
  expect(screen.getByRole("link", { name: "Cedar review · 21 oct" }).getAttribute("href")).toBe(
    "#/vault/n/Sources%2Fcedar-review.md",
  );
  expect(screen.queryByText("/Sources/cedar-review.md")).toBeNull();
});

it("renders the applied pill’s log with catalogue actions and readable authors", async () => {
  const base = fixturePort();
  window.location.hash = "#/vault/n/log.md";
  render(
    <VaultView
      port={async (call) => {
        if (call.call === "read" && call.request.path === "log.md") {
          const sample = await base({
            ...call,
            request: { ...call.request, path: "Projects/garden-pilot.md" },
          });
          if (sample.outcome === "note")
            return {
              ...sample,
              note: {
                ...sample.note,
                path: "log.md",
                body: "# Log\n\n- 11:52 · create · [Cedar review · October](/Meetings/cedar-review.md) · orbit-hub/0.14.0 · Meeting distillation\n",
                frontmatter: { title: "Registro" },
                citations: [],
                history: [],
              },
            };
        }
        return base(call);
      }}
    />,
  );
  expect(await screen.findByRole("link", { name: "Cedar review · October" })).toBeTruthy();
  expect(screen.getByText(/Creó/)).toBeTruthy();
  expect(screen.queryByText(/orbit-hub\/0.14.0/)).toBeNull();
});

it("refreshes source counts and removes the applied proposal from To review", async () => {
  const base = proposalPort({ ...proposal(), conflicts: [] });
  let applied = false;
  const port: VaultPort = async (call) => {
    if (call.call === "distillation_action" && call.action === "apply") applied = true;
    const answer = await base(call);
    return answer.outcome === "catalog"
      ? { ...answer, catalog: { ...answer.catalog, sources_count: applied ? 2 : 1 } }
      : answer;
  };
  window.location.hash = "#/vault/distill/cedar-proposal";
  render(<VaultView port={port} />);
  fireEvent.click(await screen.findByRole("button", { name: "Aplicar 3 cambios" }));
  await screen.findByRole("button", { name: "Deshacer" });
  expect(await screen.findByText(/2 fuentes/)).toBeTruthy();
  expect(screen.queryByRole("link", { name: /Cedar review.*3 cambios/ })).toBeNull();
});

it("preserves a proposed null property when editing only its text", async () => {
  const value = proposal();
  value.changes[0]!.properties[0]!.after = null;
  const port = open(value);
  fireEvent.click(await screen.findByRole("button", { name: "Cedar Pilot · 2 cambios" }));
  fireEvent.click(screen.getByRole("button", { name: "Editar antes de aplicar" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Plan" }), {
    target: { value: "Updated text" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Listo" }));
  await waitFor(() =>
    expect(port).toHaveBeenCalledWith(
      expect.objectContaining({
        call: "review_distillation",
        review: expect.objectContaining({
          changes: { project: { properties: { kickoff: null }, lines: { "0": "Updated text" } } },
        }),
      }),
    ),
  );
});

it("lets the column reach pending proposals beyond the first bounded page", async () => {
  const base = proposalPort();
  const port: VaultPort = async (call) =>
    call.call === "list_distillations" && call.page === 1
      ? { outcome: "distillations", page: { entries: [], next_page: 2 } }
      : base(call);
  render(<VaultView port={port} />);
  fireEvent.click(await screen.findByRole("button", { name: "Más propuestas" }));
  expect(await screen.findByRole("link", { name: "Revisar los cambios" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "Propuestas anteriores" })).toBeTruthy();
});

it("ignores an old proposal read after the route changes to another review", async () => {
  const base = proposalPort();
  let complete!: (answer: Awaited<ReturnType<VaultPort>>) => void;
  const port: VaultPort = (call) => {
    if (call.call === "distillation" && call.id === "cedar-proposal")
      return new Promise((resolve) => {
        complete = resolve;
      });
    if (call.call === "distillation" && call.id === "birch-proposal")
      return Promise.resolve({
        outcome: "distillation",
        proposal: { ...proposal(), id: "birch-proposal", title: "Birch review" },
      });
    return base(call);
  };
  window.location.hash = "#/vault/distill/cedar-proposal";
  render(<VaultView port={port} />);
  await waitFor(() => expect(complete).toBeTypeOf("function"));
  window.location.hash = "#/vault/distill/birch-proposal";
  await screen.findByRole("heading", { name: "Destilar «Birch review» en tus notas" });
  await act(async () => complete({ outcome: "distillation", proposal: proposal() }));
  expect(
    screen.getByRole("heading", { name: "Destilar «Birch review» en tus notas" }),
  ).toBeTruthy();
  expect(
    screen.queryByRole("heading", { name: "Destilar «Cedar review» en tus notas" }),
  ).toBeNull();
});

it("does not turn missing owner information and an invalid reviewer into Reviewed by you", async () => {
  const base = fixturePort();
  window.location.hash = "#/vault/n/Projects%2Fgarden-pilot.md";
  render(
    <VaultView
      port={async (call) => {
        const answer = await base(call);
        return answer.outcome === "note"
          ? {
              ...answer,
              note: {
                ...answer.note,
                frontmatter: { ...answer.note.frontmatter, verified: [{ by: "human:" }] },
              },
            }
          : answer;
      }}
    />,
  );
  await screen.findByRole("complementary", { name: "Sobre esta nota" });
  expect(screen.queryByText(/Revisada por ti/)).toBeNull();
});

it("does not claim the transcript was read before a queued proposal has metadata", async () => {
  open({
    ...proposal(),
    status: "queued",
    title: "",
    changes: [],
    conflicts: [],
    new_source: null,
  });
  expect(
    await screen.findByRole("heading", { name: "Destilar la reunión en tus notas" }),
  ).toBeTruthy();
  expect(screen.queryByText(/leyó la transcripción|propone 0 cambios/)).toBeNull();
});

it("does not claim a source will be saved after discarding", async () => {
  open();
  fireEvent.click(await screen.findByRole("button", { name: "Descartar" }));
  await screen.findByText(/No se ha cambiado ninguna nota ni guardado ninguna fuente/);
  expect(screen.queryByText(/La reunión se guarda como fuente\./)).toBeNull();
  expect(screen.queryByText("Se guarda como fuente")).toBeNull();
});

it("the home card waits for a ready proposal before asking for review", async () => {
  render(
    <VaultView
      port={proposalPort({
        ...proposal(),
        status: "preparing",
        title: "",
        changes: [],
        conflicts: [],
        new_source: null,
      })}
    />,
  );
  await screen.findByRole("heading", { name: "Proyectos" });
  expect(screen.queryByRole("link", { name: "Revisar los cambios" })).toBeNull();
  expect(screen.queryByText(/destiló «»|leyó la transcripción|propone 0 cambios/)).toBeNull();
});

it("shows source names in a proposed Markdown line without exposing their paths", async () => {
  const value = proposal();
  value.changes[0]!.lines[0]!.after =
    "See [Garden review](/Sources/Meetings/2026-10-21-garden-review.md).";
  open(value);
  fireEvent.click(await screen.findByRole("button", { name: "Cedar Pilot · 2 cambios" }));
  expect(screen.getByText("Garden review")).toBeTruthy();
  expect(screen.queryByText(/\/Sources\/Meetings\/2026-10-21-garden-review.md/)).toBeNull();
});
