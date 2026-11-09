import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { ChatPending } from "../../bridge/generated/ChatPending";
import { PendingCard } from "./PendingCard";

// A command that deletes is approved one at a time: the hub offers only
// «Permitir una vez» and «No» for it, and the card says why and what it
// deletes. It never offers «En esta conversación» or «Siempre».

const send = () => vi.fn<(command: AppCommand) => Promise<boolean>>(async () => true);

function deleting(deletes: { paths?: number }, choices = ["once", "deny"]): ChatPending {
  return {
    kind: "approval",
    id: "borrar-1",
    text: "rm tarifas-semana-41.csv tarifas-semana-42.csv",
    choices,
    deletes,
  };
}

describe("a command that deletes", () => {
  it("offers only once or no, and says why and how many files", async () => {
    const sent = send();
    render(<PendingCard pending={deleting({ paths: 2 })} send={sent} />);
    const card = screen.getByRole("region");
    expect(
      within(card)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["Permitir una vez", "No permitir"]);
    expect(within(card).getByText("Borra 2 archivos. No se puede deshacer.")).toBeTruthy();
    expect(
      within(card).getByText("Un comando que borra solo se permite de uno en uno."),
    ).toBeTruthy();
    expect(within(card).getByText("rm tarifas-semana-41.csv tarifas-semana-42.csv")).toBeTruthy();
    await userEvent.click(within(card).getByRole("button", { name: "Permitir una vez" }));
    expect(sent).toHaveBeenCalledWith({ command: "approval", approved: true, choice: "once" });
  });

  it("says one file in the singular, and files without a count it does not know", () => {
    const { unmount } = render(<PendingCard pending={deleting({ paths: 1 })} send={send()} />);
    expect(screen.getByText("Borra 1 archivo. No se puede deshacer.")).toBeTruthy();
    unmount();
    render(<PendingCard pending={deleting({})} send={send()} />);
    expect(screen.getByText("Borra archivos. No se puede deshacer.")).toBeTruthy();
  });

  it("never shows a wider choice, even if one came with it", () => {
    render(
      <PendingCard
        pending={deleting({ paths: 2 }, ["once", "deny", "session", "always"])}
        send={send()}
      />,
    );
    expect(screen.queryByRole("button", { name: "En esta conversación" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Siempre" })).toBeNull();
  });

  it("an approval that deletes nothing keeps its choices and says nothing of deleting", () => {
    render(
      <PendingCard
        pending={{
          kind: "approval",
          id: "listar-1",
          text: "ls -la",
          choices: ["once", "deny", "session", "always"],
        }}
        send={send()}
      />,
    );
    expect(screen.getByRole("button", { name: "Siempre" })).toBeTruthy();
    expect(screen.queryByText(/No se puede deshacer/)).toBeNull();
  });
});
