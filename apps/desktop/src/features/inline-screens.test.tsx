import { Sidebar } from "../app/Sidebar";
import { appView, notesSnapshot } from "./notes/test-notes";
import { ConnectionCard } from "./vault/ConnectionCard";
import { FormatSettings } from "./vault/VaultFormatSettings";
import { NoteProperties } from "./vault/NoteProperties";
import fixture from "./vault/__fixtures__/reading.json";
import type { VaultNote } from "../bridge/generated/VaultNote";
import type { VaultSettings } from "../bridge/generated/VaultSettings";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { CopyProvider } from "../i18n/CopyProvider";
import { spanish } from "../i18n/es";
import { english } from "../i18n/en";
import { configureCopy } from "../i18n";
import { DAY_MS, monthTitle } from "./calendar/date";
import VaultView from "./vault/VaultView";
import { VaultSettingsView } from "./vault/VaultSettingsView";
import { fixturePort } from "./vault/test-port";
import { AIStep } from "./onboarding/OnboardingAIStep";
import { AISettings } from "./settings/AISettings";
import type { AgentPort } from "./settings/agents/use-agent-config";
import { dayOf } from "./assistant/ConversationList";
import { sessionWhen } from "./assistant/session-title";
import { workSummary } from "./assistant/AgentWork";

function strings(value: unknown): string[] {
  if (typeof value === "string") return [value];
  return value !== null && typeof value === "object" ? Object.values(value).flatMap(strings) : [];
}
function expectEnglish(container: HTMLElement) {
  const en = new Set(strings(english("Atlas")));
  const forbidden = new Set(strings(spanish("Atlas")).filter((text) => !en.has(text)));
  const walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT);
  while (walker.nextNode())
    expect(forbidden.has(walker.currentNode.textContent?.trim() ?? "")).toBe(false);
  for (const node of container.querySelectorAll("*"))
    for (const name of ["aria-label", "title", "placeholder"])
      expect(forbidden.has(node.getAttribute(name) ?? "")).toBe(false);
}
afterEach(() => {
  cleanup();
  window.location.hash = "";
  configureCopy("es", ["es-ES"], "Deimos");
});

it.each([
  ["#/vault", "Projects"],
  ["#/vault/tasks", "Tasks"],
  ["#/vault/map", "Map"],
  ["#/vault/n/People%2Fada-vale.md", "Ada Vale"],
  ["#/vault/n/Projects%2Fgarden-pilot.md", "Garden Pilot"],
])("renders the English vault at %s, including accessible copy", async (route, title) => {
  window.location.hash = route;
  const view = render(
    <CopyProvider choice="en">
      <VaultView port={fixturePort()} />
    </CopyProvider>,
  );
  await screen.findByRole("heading", { name: title });
  if (route.includes("/tasks")) {
    expect(await screen.findByRole("button", { name: /^Overdue/ })).toBeTruthy();
    expect(screen.getAllByRole("checkbox")[0]?.getAttribute("title")).toBe(
      "Read only: checking off tasks is coming later",
    );
  }
  if (route.includes("/map"))
    expect(
      await screen.findByRole("button", { name: /Ada Vale · Person · \d+ links/ }),
    ).toBeTruthy();
  if (route.includes("Projects%2F"))
    expect(await screen.findByRole("img", { name: "Dependency diagram" })).toBeTruthy();
  expectEnglish(view.container);
});

it("renders vault connection, format and future settings in English", async () => {
  const port = fixturePort();
  const view = render(
    <CopyProvider choice="en">
      <VaultSettingsView port={port} />
    </CopyProvider>,
  );
  await screen.findByRole("button", { name: "Save configuration" });
  expect(screen.getByLabelText("Path on the hub").getAttribute("placeholder")).toBe(
    "/absolute/path/to/vault",
  );
  fireEvent.change(screen.getByLabelText("Connector"), { target: { value: "fast_note_sync" } });
  expect(screen.getByLabelText("Fast Note Sync URL")).toBeTruthy();
  expect(screen.getByRole("switch", { name: "Apply automatically and notify me" })).toBeTruthy();
  expectEnglish(view.container);
});

const aiPort = vi.fn(async (call) => ({
  call,
  outcome: {
    outcome: "ai_settings",
    settings: {
      providers: [
        {
          id: "local",
          name: "Cedar",
          kind: "local",
          base_url: "https://ai.example.test",
          key_configured: true,
          models: [{ id: "cedar", capabilities: ["chat"] }],
        },
      ],
      functions: [
        {
          id: "assistants",
          provider_id: "local",
          model: "cedar",
          capability: "chat",
          privacy: "Runs on the hub.",
        },
      ],
    },
  },
})) as AgentPort;
it("renders AI settings, model capabilities and the provider editor in English", async () => {
  const view = render(
    <CopyProvider choice="en">
      <AISettings agent={aiPort} />
    </CopyProvider>,
  );
  await screen.findByText("Cedar");
  fireEvent.click(screen.getByRole("button", { name: "Edit Cedar" }));
  expect(screen.getByLabelText("Provider type")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Save capabilities" })).toBeTruthy();
  expectEnglish(view.container);
});
it("renders the onboarding AI choices and provider form in English", () => {
  const view = render(
    <CopyProvider choice="en">
      <AIStep agent={aiPort} />
    </CopyProvider>,
  );
  fireEvent.click(screen.getByRole("radio", { name: "Add a provider" }));
  expect(screen.getByLabelText("Provider type")).toBeTruthy();
  expectEnglish(view.container);
});
it("translates helper-generated dates and work summaries at the point of use", () => {
  const at = new Date(2026, 9, 21, 9, 30).getTime();
  configureCopy("en", ["en-GB"], "Atlas");
  expect(monthTitle(Date.UTC(2026, 9, 1) / DAY_MS)).toBe("October 2026");
  expect(dayOf(at, at)).toBe("Today");
  expect(sessionWhen(at, at)).toBe("Today, 09:30");
  expect(sessionWhen(at - 86_400_000, at)).toBe("Yesterday, 09:30");
  expect(
    workSummary({
      tools: 2,
      subjects: ["Mail", "Calendar"],
      started_at_ms: 0,
      ended_at_ms: 72_000,
    }),
  ).toContain("Mail and Calendar");
});

it("renders the Notes navigation label in the English shell", () => {
  const value = appView(notesSnapshot({ list: [] }));
  const snapshot = {
    ...value,
    chat: { ...value.chat, pending: null },
    meeting: { ...value.meeting, phase: "idle" as const },
    call: null,
  };
  const view = render(
    <CopyProvider choice="en">
      <Sidebar snapshot={snapshot} view="vault" send={() => undefined} onRecord={() => undefined} />
    </CopyProvider>,
  );
  expect(screen.getByRole("link", { name: "Notes" }).getAttribute("aria-current")).toBe("page");
  expectEnglish(view.container);
});

it("rewords vault properties, retained save feedback and drafts without remounting", async () => {
  const port = fixturePort();
  const { configuration } = (await port({ call: "config" })) as { configuration: VaultSettings };
  const note = fixture.documents["Projects/garden-pilot.md"] as VaultNote;
  const saved = vi.fn();
  const content = (choice: "es" | "en") => (
    <CopyProvider choice={choice}>
      <ConnectionCard configuration={configuration} onSaved={saved} port={port} />
      <FormatSettings configuration={configuration} saved={saved} port={port} />
      <NoteProperties note={note} notes={[]} fieldNames={{}} />
    </CopyProvider>
  );
  const view = render(content("es"));
  fireEvent.change(screen.getByLabelText("Ruta en el hub"), {
    target: { value: "/invented/Draft" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Guardar configuración" }));
  await screen.findByText("Guardado en el hub.");
  view.rerender(content("en"));
  expect(screen.getByText("Saved in the hub.")).toBeTruthy();
  expect(screen.getByText("Project stage")).toBeTruthy();
  expect((screen.getByLabelText("Path on the hub") as HTMLInputElement).value).toBe(
    "/invented/Draft",
  );
  expectEnglish(view.container);
});
