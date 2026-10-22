import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { CorrectionDialog } from "./CorrectionDialog";

// «Corregir texto»: from the press the dialog says it is saving, until the
// term is seen saved (it closes) or the order is refused (it can be pressed
// again). It never sits there as if nothing had been pressed.

const note = {
  recording_id: "rec-taller",
  title: "Taller de producto",
  dictionary: { dictionary_id: "dic-1", series: null },
} as unknown as OpenNoteSnapshot;
const library = {
  dictionaries: { state: "available", detail: null },
  dictionary_items: [{ dictionary: { id: "dic-1", name: "Producto", terms: [] }, pending: false }],
  dictionary_pending: [],
  dictionary_failures: [],
  read: "available",
} as unknown as NoteLibrarySnapshot;

describe("saving a correction", () => {
  it("says it is saving from the press", async () => {
    const send = vi.fn<(command: AppCommand) => Promise<boolean>>(async () => true);
    render(
      <CorrectionDialog
        library={library}
        note={note}
        onClose={() => undefined}
        send={send}
        wrong="Orvit"
      />,
    );
    await userEvent.clear(screen.getByLabelText("Forma correcta"));
    await userEvent.type(screen.getByLabelText("Forma correcta"), "Orbit");
    await userEvent.click(screen.getByRole("button", { name: "Guardar corrección" }));
    expect(send).toHaveBeenCalledTimes(1);
    expect(screen.getByText("Guardando en el hub…")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Guardar corrección" })).toHaveProperty(
      "disabled",
      true,
    );
  });

  it("can be pressed again when the order is refused", async () => {
    const send = vi.fn<(command: AppCommand) => Promise<boolean>>(async () => false);
    render(
      <CorrectionDialog
        library={library}
        note={note}
        onClose={() => undefined}
        send={send}
        wrong="Orvit"
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Guardar corrección" }));
    await waitFor(() => expect(screen.queryByText("Guardando en el hub…")).toBeNull());
    expect(screen.getByRole("button", { name: "Guardar corrección" })).toHaveProperty(
      "disabled",
      false,
    );
  });
});
