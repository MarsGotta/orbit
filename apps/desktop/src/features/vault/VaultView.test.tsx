import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { VaultPort } from "./api";
import { fixturePort } from "./test-port";
import VaultView from "./VaultView";
import { VaultSettingsView } from "./VaultSettingsView";

afterEach(() => {
  cleanup();
  window.location.hash = "";
});
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

it("52 explains connection, validates settings and reaches the reading column", async () => {
  let connected = false;
  const reading = fixturePort();
  const port = vi.fn<VaultPort>(async (call) => {
    if (call.call === "config")
      return { outcome: "config", configuration: { ...configuration, configured: connected } };
    if (call.call === "configure") {
      connected = true;
      return { outcome: "config", configuration };
    }
    return reading(call);
  });
  render(<VaultView port={port} />);
  expect(await screen.findByRole("heading", { name: "Tus notas, en Markdown" })).not.toBeNull();
  await screen.findByLabelText("Ruta en el hub");
  fireEvent.change(screen.getByLabelText("Ruta en el hub"), {
    target: { value: "/invented/Garden" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Comprobar y conectar" }));
  await screen.findByRole("complementary", { name: "Tu vault" });
  fireEvent.click(await screen.findByRole("link", { name: "Ada Vale" }));
  expect(await screen.findByRole("heading", { name: "Ada Vale" })).not.toBeNull();
  expect(await screen.findByRole("complementary", { name: "Sobre esta nota" })).not.toBeNull();
});

it("keeps a settings refusal in place and clears the token after submitting", async () => {
  const port = vi.fn<VaultPort>(async (call) =>
    call.call === "config"
      ? { outcome: "config", configuration: { ...configuration, configured: false } }
      : {
          outcome: "refused",
          code: "vault_token_expired",
          message: "UNTRUSTED BACKEND MESSAGE",
          numbers: { upstream_code: 508 },
        },
  );
  render(<VaultView port={port} />);
  await screen.findByLabelText("Ruta en el hub");
  fireEvent.change(screen.getByLabelText("Conector"), { target: { value: "fast_note_sync" } });
  fireEvent.change(screen.getByLabelText("URL de Fast Note Sync"), {
    target: { value: "https://sync.example.test" },
  });
  fireEvent.change(screen.getByLabelText("Nombre del vault"), { target: { value: "Invented" } });
  fireEvent.change(screen.getByLabelText("Token"), { target: { value: "invented-token" } });
  fireEvent.click(screen.getByRole("button", { name: "Comprobar y conectar" }));
  expect((await screen.findByRole("alert")).textContent).toContain(
    "El token de Fast Note Sync ha caducado.",
  );
  await waitFor(() => expect((screen.getByLabelText("Token") as HTMLInputElement).value).toBe(""));
});

it("64 stores link style and links to the AI model choice while maintenance stays inert", async () => {
  const port = vi.fn<VaultPort>(async (call) => ({
    outcome: "config",
    configuration:
      call.call === "configure" ? { ...configuration, ...call.configuration } : configuration,
  }));
  render(<VaultSettingsView port={port} />);
  fireEvent.click(await screen.findByLabelText("[[Wikilinks]]"));
  expect(
    screen.getByRole("link", { name: "Elegir modelo en Ajustes › IA" }).getAttribute("href"),
  ).toBe("#/settings/ia");
  fireEvent.click(screen.getByRole("button", { name: "Guardar configuración" }));
  await waitFor(() =>
    expect(port).toHaveBeenCalledWith(
      expect.objectContaining({
        call: "configure",
        configuration: expect.objectContaining({
          link_style: "wikilink",
          maintenance_model: "",
        }),
      }),
    ),
  );
  expect(
    screen.getAllByRole("switch").every((control) => (control as HTMLButtonElement).disabled),
  ).toBe(true);
  expect(screen.queryByRole("button", { name: "Revisar ahora" })).toBeNull();
});
