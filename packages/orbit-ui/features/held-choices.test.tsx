import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../bridge/generated";
import type { ChatPending } from "../bridge/generated/ChatPending";
import type { OpenNoteSnapshot } from "../bridge/generated/OpenNoteSnapshot";
import { PendingCard } from "./assistant/PendingCard";
import { ActionsTab } from "./notes/ActionsTab";
import { CallPreferenceList } from "./settings/RecordingSection";

// A control shows what the person chose while its order travels and until
// the hub confirms it; it does not snap back in between. An answer to Deimos
// is not given twice while the first one is on its way.

const accepted = () => vi.fn<(command: AppCommand) => Promise<boolean>>(async () => true);
const refused = () => vi.fn<(command: AppCommand) => Promise<boolean>>(async () => false);

describe("a choice kept until it is confirmed", () => {
  it("«Cuando empieces una videollamada» stays where the person put it", async () => {
    const send = accepted();
    render(<CallPreferenceList preference="notify" send={send} />);
    await userEvent.click(screen.getByRole("radio", { name: /^Grabar sin preguntar/ }));
    expect(send).toHaveBeenCalledWith({ command: "call_preference", preference: "record" });
    expect(screen.getByRole("radio", { name: /^Grabar sin preguntar/ })).toHaveProperty(
      "checked",
      true,
    );
  });

  it("…and goes back when the order is refused", async () => {
    render(<CallPreferenceList preference="notify" send={refused()} />);
    await userEvent.click(screen.getByRole("radio", { name: /^Grabar sin preguntar/ }));
    await waitFor(() =>
      expect(screen.getByRole("radio", { name: /^Avisarme \(recomendado\)/ })).toHaveProperty(
        "checked",
        true,
      ),
    );
  });

  it("a task's box stays ticked until the note is read again", async () => {
    const note = {
      recording_id: "rec-taller",
      owner_name: "Lucía Prado",
      summary: "Resumen",
      actions: [{ id: "a1", text: "Enviar el presupuesto", owner: "Lucía Prado", done: false }],
    } as unknown as OpenNoteSnapshot;
    const send = accepted();
    render(<ActionsTab note={note} send={send} />);
    await userEvent.click(screen.getByRole("checkbox"));
    expect(send).toHaveBeenCalledWith(
      expect.objectContaining({ command: "note_action_toggle", action_id: "a1", done: true }),
    );
    // The design's checkbox (design/Checkbox.tsx) is a button with aria-checked.
    expect(screen.getByRole("checkbox").getAttribute("aria-checked")).toBe("true");
  });
});

describe("an answer to Deimos on its way", () => {
  const approval: ChatPending = {
    kind: "approval",
    id: "req-1",
    text: "¿Leer el calendario?",
    choices: [],
  };

  it("is not given twice: the card waits for the hub", async () => {
    const send = accepted();
    render(<PendingCard pending={approval} send={send} />);
    await userEvent.click(screen.getByRole("button", { name: "Permitir una vez" }));
    expect(send).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Permitir una vez" })).toHaveProperty(
      "disabled",
      true,
    );
    expect(screen.getByRole("button", { name: "No permitir" })).toHaveProperty("disabled", true);
  });

  it("can be given again when the answer was refused", async () => {
    render(<PendingCard pending={approval} send={refused()} />);
    await userEvent.click(screen.getByRole("button", { name: "Permitir una vez" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Permitir una vez" })).toHaveProperty(
        "disabled",
        false,
      ),
    );
  });

  it("the next question of a clarification is answerable at once", async () => {
    const clarify = (qid: string): ChatPending => ({
      kind: "clarify",
      id: "req-2",
      text: "Dos preguntas",
      choices: [],
      questions: [{ qid, question: `Pregunta ${qid}`, choices: ["Sí", "No"], multi_select: false }],
    });
    const send = accepted();
    const view = render(<PendingCard pending={clarify("q1")} send={send} />);
    await userEvent.click(screen.getByRole("button", { name: "Sí" }));
    expect(screen.getByRole("button", { name: "Sí" })).toHaveProperty("disabled", true);
    view.rerender(<PendingCard pending={clarify("q2")} send={send} />);
    expect(screen.getByRole("button", { name: "Sí" })).toHaveProperty("disabled", false);
  });
});
