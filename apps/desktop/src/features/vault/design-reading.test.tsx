import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { fixturePort } from "./test-port";
import VaultView from "./VaultView";

afterEach(() => {
  cleanup();
  window.location.hash = "";
});

it("51 has the vault column, projects, tasks and a keyboard path to the reading screens", async () => {
  window.location.hash = "#/vault";
  render(<VaultView port={fixturePort()} />);
  expect(await screen.findByRole("complementary", { name: "Tu vault" })).not.toBeNull();
  expect(await screen.findByRole("heading", { name: "Proyectos" })).not.toBeNull();
  const column = screen.getByRole("complementary", { name: "Tu vault" });
  expect(within(column).getByRole("link", { name: "Mapa" })).not.toBeNull();
  expect(screen.queryByRole("button", { name: /Destilar|Nueva nota/ })).toBeNull();
});

it("54–56 show properties, citation sources, raw Markdown and project dependencies", async () => {
  window.location.hash = "#/vault/n/personas%2Fjavier-ortega.md";
  render(<VaultView port={fixturePort()} />);
  const panel = await screen.findByRole("complementary", { name: "Sobre esta nota" });
  expect(within(panel).getByRole("heading", { name: "Fuentes · 3" })).not.toBeNull();
  expect(await screen.findByText("Backend de reservas")).not.toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "MD Markdown" }));
  expect(await screen.findByText(/Propiedades \(YAML\) · OKF 0.2/)).not.toBeNull();
  expect(screen.getAllByText(/type: persona/).length).toBeGreaterThan(0);
  window.location.hash = "#/vault/n/proyectos%2Fpiloto-hoteles-ribera.md";
  fireEvent(window, new Event("hashchange"));
  expect(await screen.findByRole("img", { name: "Diagrama de dependencias" })).not.toBeNull();
  expect(await screen.findByText("Preparar la demo con Ribera Aeropuerto")).not.toBeNull();
  expect(
    screen.getAllByRole("checkbox").every((input) => (input as HTMLInputElement).disabled),
  ).toBe(true);
});

it("57 is read-only and 58 exposes each map note to keyboard users", async () => {
  window.location.hash = "#/vault/tasks";
  render(<VaultView port={fixturePort()} />);
  await screen.findByText("Confirmar con Pablo la fecha de arranque");
  expect(
    screen.getAllByRole("checkbox").every((input) => (input as HTMLInputElement).disabled),
  ).toBe(true);
  fireEvent.click(screen.getByRole("radio", { name: "Por proyecto" }));
  expect(await screen.findByRole("heading", { name: /Piloto Hoteles Ribera/ })).not.toBeNull();
  window.location.hash = "#/vault/map";
  fireEvent(window, new Event("hashchange"));
  const node = await screen.findByRole("button", { name: /Javier Ortega · Persona/ });
  fireEvent.keyDown(node, { key: "Enter" });
  await waitFor(() => expect(node.getAttribute("aria-pressed")).toBe("true"));
  expect(screen.getByText(/Lista equivalente del mapa/)).not.toBeNull();
});

it("the folder tree expands nested source folders without flattening their paths", async () => {
  window.location.hash = "#/vault";
  render(<VaultView port={fixturePort()} />);
  const column = await screen.findByRole("complementary", { name: "Tu vault" });
  fireEvent.click(await within(column).findByRole("button", { name: /Fuentes/ }));
  const folder = await within(column).findByRole("button", { name: /reuniones/i });
  fireEvent.click(folder);
  expect(
    within(column).getByRole("link", { name: "Revisión del diseño de reservas · 21 oct" }),
  ).not.toBeNull();
});

it("shows deprecated notes as retired in their reading properties", async () => {
  window.location.hash = "#/vault/n/personas%2Fjavier-ortega.md";
  const fixture = fixturePort();
  render(
    <VaultView
      port={async (call) => {
        const result = await fixture(call);
        return result.outcome === "note"
          ? {
              ...result,
              note: {
                ...result.note,
                frontmatter: { ...result.note.frontmatter, status: "deprecated" },
              },
            }
          : result;
      }}
    />,
  );
  expect(await screen.findByText("Retirada")).not.toBeNull();
});

it("treats an ordinary code fence using the internal task language as literal text", async () => {
  window.location.hash = "#/vault/n/personas%2Fjavier-ortega.md";
  const fixture = fixturePort();
  const body = "## Example\n\n```orbit-tasks\nInvented literal code\n```\n";
  render(
    <VaultView
      port={async (call) => {
        const result = await fixture(call);
        return result.outcome === "note"
          ? { ...result, note: { ...result.note, body, raw: body } }
          : result;
      }}
    />,
  );
  expect(await screen.findByText("Invented literal code")).not.toBeNull();
});

it("keeps notes with no links in a separate area of the map", async () => {
  window.location.hash = "#/vault/map";
  const fixture = fixturePort();
  render(
    <VaultView
      port={async (call) => {
        const result = await fixture(call);
        if (result.outcome !== "catalog") return result;
        const first = result.catalog.notes[0];
        if (first === undefined) return result;
        const loose = {
          ...first,
          path: "loose.md",
          title: "Invented loose note",
          type: "nota",
          links: [],
          backlink_count: 0,
        };
        return {
          ...result,
          catalog: { ...result.catalog, notes: [...result.catalog.notes, loose] },
        };
      }}
    />,
  );
  expect(await screen.findByText("Notas sueltas")).not.toBeNull();
  expect(
    screen.getByRole("button", { name: /Invented loose note/ }).getAttribute("data-isolated"),
  ).toBe("true");
});
