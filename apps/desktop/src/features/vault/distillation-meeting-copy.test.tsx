import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { CopyProvider } from "orbit-ui/i18n/CopyProvider";
import VaultView from "./VaultView";
import { proposal, proposalPort } from "./test-distillation";

afterEach(() => {
  cleanup();
  window.location.hash = "";
});

it.each([
  [
    "es",
    "Lo que se dijo en la reunión",
    "La nota no cambia. Lo que se dijo en la reunión queda en la fuente.",
  ],
  [
    "en",
    "What was said in the meeting",
    "The note stays unchanged. What was said in the meeting stays in the source.",
  ],
] as const)(
  "describes an older meeting without calling it today in %s",
  async (locale, heading, help) => {
    window.location.hash = "#/vault/distill/cedar-proposal";
    render(
      <CopyProvider choice={locale}>
        <VaultView port={proposalPort({ ...proposal(), created_at: "2020-10-21T11:52:00Z" })} />
      </CopyProvider>,
    );
    expect(await screen.findByRole("heading", { name: heading })).toBeTruthy();
    expect(screen.getByText(help)).toBeTruthy();
  },
);
