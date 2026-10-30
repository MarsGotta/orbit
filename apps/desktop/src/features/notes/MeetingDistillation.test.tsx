import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import NotesView from "./NotesView";
import { appView, notesSnapshot, writtenNote } from "./test-notes";
import { proposal, proposalPort } from "../vault/test-distillation";
afterEach(() => {
  cleanup();
  window.location.hash = "";
});
it.each(["summary", "transcript", "private"])(
  "the amber pill stays in the meeting on %s",
  async (tab) => {
    const port = proposalPort();
    render(
      <NotesView
        error={null}
        onRecord={() => undefined}
        send={async () => undefined}
        snapshot={appView(notesSnapshot({ open: writtenNote() }))}
        vault={port}
      />,
    );
    if (tab !== "summary")
      fireEvent.click(
        screen.getByRole("tab", {
          name: tab === "transcript" ? "Transcripción" : "Notas privadas",
        }),
      );
    const pill = await screen.findByRole("link", { name: "Revisar el destilado · 3 cambios" });
    expect(pill.getAttribute("href")).toBe("#/vault/distill/cedar-proposal");
    expect(pill.getAttribute("title")).toContain("2 notas");
    expect(pill.getAttribute("data-applied")).toBe("false");
  },
);
it("the green pill opens the log after human application", async () => {
  render(
    <NotesView
      error={null}
      onRecord={() => undefined}
      send={async () => undefined}
      snapshot={appView(notesSnapshot({ open: writtenNote() }))}
      vault={proposalPort({ ...proposal(), status: "applied" })}
    />,
  );
  const pill = await screen.findByRole("link", { name: "En Notas · 3 cambios" });
  expect(pill.getAttribute("href")).toBe("#/vault/n/log.md");
  expect(pill.getAttribute("data-applied")).toBe("true");
});
it("offers on-demand distillation for an older ready meeting only when a model and vault are available", async () => {
  const base = proposalPort();
  const port: typeof base = vi.fn(async (call) => {
    if (call.call === "list_distillations")
      return { outcome: "distillations", page: { entries: [], next_page: null } };
    const result = await base(call);
    if (result.outcome === "config")
      return { ...result, configuration: { ...result.configuration, maintenance_enabled: true } };
    return result;
  });
  render(
    <NotesView
      error={null}
      onRecord={() => undefined}
      send={async () => undefined}
      snapshot={appView(notesSnapshot({ open: writtenNote() }))}
      vault={port}
    />,
  );
  fireEvent.click(await screen.findByRole("button", { name: "Destilar en Notas" }));
  await waitFor(() =>
    expect(port).toHaveBeenCalledWith({
      call: "prepare_distillation",
      meeting_id: writtenNote().recording_id,
    }),
  );
});

it("keeps distillation absent on a Mac without a connected vault", async () => {
  const base = proposalPort();
  const port: typeof base = vi.fn(async (call) => {
    if (call.call === "list_distillations")
      return { outcome: "refused", code: "vault_not_connected", message: "synthetic", numbers: {} };
    const answer = await base(call);
    return answer.outcome === "config"
      ? { ...answer, configuration: { ...answer.configuration, configured: false } }
      : answer;
  });
  render(
    <NotesView
      error={null}
      onRecord={() => undefined}
      send={async () => undefined}
      snapshot={appView(notesSnapshot({ open: writtenNote() }))}
      vault={port}
    />,
  );
  await waitFor(() => expect(port).toHaveBeenCalledWith({ call: "config" }));
  expect(port.mock.calls.some(([call]) => call.call === "list_distillations")).toBe(false);
  expect(screen.queryByRole("button", { name: "Destilar en Notas" })).toBeNull();
  expect(screen.queryByRole("alert")).toBeNull();
});
