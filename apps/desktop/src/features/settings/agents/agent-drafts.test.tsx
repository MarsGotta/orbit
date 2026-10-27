import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentAnswer } from "../../../bridge/generated/AgentAnswer";
import type { AgentCall } from "../../../bridge/generated/AgentCall";
import type { AgentConfig } from "../../../bridge/generated/AgentConfig";
import { AgentCards } from "./AgentCards";

// A personality or a skill written and not saved yet stays on this Mac when
// the person goes to another view, and comes back where it was written.
// The Markdown editor is Tiptap's, tested on its own; a text box stands in.
vi.mock("./PersonalityEditor", () => ({
  default: ({
    label,
    markdown,
    onChange,
  }: {
    label: string;
    markdown: string;
    onChange: (markdown: string) => void;
  }) => (
    <textarea
      aria-label={label}
      defaultValue={markdown}
      onChange={(event) => onChange(event.currentTarget.value)}
    />
  ),
}));

const VEGA = { id: "vega", name: "Vega" };
const SKILL_TEXT = "---\nname: resumen-diario\ndescription: Resume el día.\n---\n\nPasos.";

function config(): AgentConfig {
  return {
    agent_id: "vega",
    kind: "openai",
    personality: { text: "Sé breve.", editable: true, max_bytes: 65536, cause: null },
    access: {
      groups: {
        screen: true,
        control: false,
        calendar: true,
        meetings: true,
        files: true,
        notes: true,
      },
      editable: true,
      cause: null,
    },
    skills: {
      editable: true,
      cause: null,
      max_skills: 8,
      max_skill_bytes: 4096,
      items: [{ name: "resumen-diario", description: "Resume el día.", enabled: true }],
    },
  };
}

let calls: AgentCall[];
const agent = vi.fn(async (call: AgentCall): Promise<AgentAnswer> => {
  calls.push(call);
  if (call.call === "config") return { call, outcome: { outcome: "config", config: config() } };
  if (call.call === "skill")
    return {
      call,
      outcome: {
        outcome: "skill",
        skill: {
          name: call.name,
          description: "Resume el día.",
          enabled: true,
          content: SKILL_TEXT,
        },
      },
    };
  return { call, outcome: { outcome: "done" } };
});

beforeEach(() => {
  calls = [];
  window.localStorage.clear();
});
afterEach(cleanup);

async function openSettings() {
  const view = render(<AgentCards agent={agent} target={VEGA} />);
  await screen.findByRole("switch", { name: "Pantalla" });
  return view;
}

const personality = () =>
  screen.getByRole("textbox", { name: "Personalidad de Vega" }) as HTMLTextAreaElement;

async function openSkill() {
  fireEvent.click(screen.getByRole("button", { name: "Editar resumen-diario" }));
  return (await screen.findByRole("textbox", {
    name: "Contenido del SKILL.md",
  })) as HTMLTextAreaElement;
}

describe("an agent's drafts", () => {
  it("a personality written and not saved is still there after visiting another view", async () => {
    const first = await openSettings();
    fireEvent.change(personality(), { target: { value: "Sé breve y amable." } });
    first.unmount();
    await openSettings();
    expect(personality().value).toBe("Sé breve y amable.");
    expect(screen.getByRole("button", { name: "Guardar" }).hasAttribute("disabled")).toBe(false);
  });

  it("a personality the hub took is no longer kept as a draft", async () => {
    const first = await openSettings();
    fireEvent.change(personality(), { target: { value: "Sé breve y amable." } });
    fireEvent.click(screen.getByRole("button", { name: "Guardar" }));
    await waitFor(() => expect(calls.some((call) => call.call === "personality")).toBe(true));
    await screen.findByText("✓ Guardado en el hub");
    first.unmount();
    await openSettings();
    expect(personality().value).toBe("Sé breve.");
  });

  it("a skill being edited is still there after visiting another view", async () => {
    const first = await openSettings();
    fireEvent.change(await openSkill(), { target: { value: `${SKILL_TEXT}\nUno más.` } });
    first.unmount();
    await openSettings();
    expect((await openSkill()).value).toBe(`${SKILL_TEXT}\nUno más.`);
  });

  it("«Cancelar» lets the skill's draft go", async () => {
    await openSettings();
    fireEvent.change(await openSkill(), { target: { value: `${SKILL_TEXT}\nUno más.` } });
    fireEvent.click(screen.getByRole("button", { name: "Cancelar" }));
    expect((await openSkill()).value).toBe(SKILL_TEXT);
  });
});
