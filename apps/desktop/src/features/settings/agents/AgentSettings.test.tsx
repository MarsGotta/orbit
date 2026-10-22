import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentAnswer } from "../../../bridge/generated/AgentAnswer";
import type { AgentCall } from "../../../bridge/generated/AgentCall";
import type { AgentConfig } from "../../../bridge/generated/AgentConfig";
import type { AgentOutcome } from "../../../bridge/generated/AgentOutcome";
import type { AppView } from "../../../state/app-view";
import SettingsView from "../SettingsView";
import { AgentSettings } from "./AgentSettings";

// The Markdown editor is Tiptap's, tested on its own (PersonalityEditor);
// here a text box stands in for it so the orders can be typed.
vi.mock("./PersonalityEditor", () => ({
  default: ({
    label,
    markdown,
    editable,
    onChange,
  }: {
    label: string;
    markdown: string;
    editable: boolean;
    onChange: (markdown: string) => void;
  }) => (
    <textarea
      aria-label={label}
      defaultValue={markdown}
      readOnly={!editable}
      onChange={(event) => onChange(event.currentTarget.value)}
    />
  ),
}));

// Ajustes › Agentes: the agents from the status topic, each one's
// configuration read from the hub, every order sent as given and every
// refusal said with its numbers. Names and contents are invented.

const AGENTS = [
  { id: "vega", name: "Vega" },
  { id: "atlas", name: "Atlas" },
];

function vegaConfig(): AgentConfig {
  return {
    agent_id: "vega",
    kind: "openai",
    personality: { text: "Sé breve.", editable: true, max_bytes: 65536, cause: null },
    access: {
      groups: { screen: true, control: false, calendar: true, meetings: true, files: true },
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

function atlasConfig(): AgentConfig {
  return {
    agent_id: "atlas",
    kind: "hermes",
    personality: {
      text: "Soy Atlas.",
      editable: false,
      max_bytes: 65536,
      cause: "La personalidad es el SOUL.md del perfil",
    },
    access: {
      groups: { screen: true, control: true, calendar: true, meetings: true, files: true },
      editable: false,
      cause: "Hermes decide sus herramientas",
    },
    skills: {
      editable: false,
      cause: null,
      max_skills: 8,
      max_skill_bytes: 4096,
      items: [{ name: "agenda", description: "Mira la agenda.", enabled: true }],
    },
  };
}

const SKILL_TEXT = "---\nname: resumen-diario\ndescription: Resume el día.\n---\n\nPasos.";

let calls: AgentCall[];
let replies: Map<string, AgentOutcome>;

function key(call: AgentCall): string {
  return call.call === "config" ? `config:${call.agent_id}` : call.call;
}

const agent = vi.fn(async (call: AgentCall): Promise<AgentAnswer> => {
  calls.push(call);
  const outcome =
    replies.get(key(call)) ??
    (call.call === "config"
      ? {
          outcome: "config" as const,
          config: call.agent_id === "atlas" ? atlasConfig() : vegaConfig(),
        }
      : call.call === "skill"
        ? {
            outcome: "skill" as const,
            skill: {
              name: call.name,
              description: "Resume el día.",
              enabled: true,
              content: SKILL_TEXT,
            },
          }
        : { outcome: "done" as const });
  return { call, outcome };
});

beforeEach(() => {
  calls = [];
  replies = new Map();
  agent.mockClear();
});
afterEach(cleanup);

async function renderVega() {
  render(<AgentSettings agents={AGENTS} defaultAgentId="vega" agent={agent} />);
  await screen.findByText("Vega se configura desde aquí. Los cambios se guardan en el hub.");
}

describe("Ajustes › Agentes", () => {
  it("reads the default agent's configuration and shows each group with what it lets the agent do", async () => {
    await renderVega();
    expect(calls[0]).toEqual({ call: "config", agent_id: "vega" });
    const section = screen.getByRole("region", { name: "Agentes" });
    expect(within(section).getByRole("button", { name: "Vega" }).getAttribute("aria-pressed")).toBe(
      "true",
    );
    expect(screen.getByRole("switch", { name: "Pantalla" }).getAttribute("aria-checked")).toBe(
      "true",
    );
    expect(screen.getByRole("switch", { name: "Usar el Mac" }).getAttribute("aria-checked")).toBe(
      "false",
    );
    expect(
      screen.getByText("Ver tus ventanas, hacer capturas y leer el texto que selecciones"),
    ).toBeTruthy();
    expect(screen.getByText("Pulsar, escribir y abrir apps por ti")).toBeTruthy();
    expect(screen.getByText("Leer tu agenda")).toBeTruthy();
    expect(screen.getByText("Leer tus reuniones, notas y transcripciones")).toBeTruthy();
    expect(screen.getByText("Leer los archivos de las carpetas que elijas")).toBeTruthy();
    expect(screen.getByText("resumen-diario")).toBeTruthy();
    expect(screen.getByText("1 de 8")).toBeTruthy();
  });

  it("a switch is the decision: the order goes at once with the five groups, and the config is read again", async () => {
    await renderVega();
    fireEvent.click(screen.getByRole("switch", { name: "Usar el Mac" }));
    await waitFor(() => expect(calls).toHaveLength(3));
    expect(calls[1]).toEqual({
      call: "access",
      agent_id: "vega",
      groups: { screen: true, control: true, calendar: true, meetings: true, files: true },
    });
    expect(calls[2]).toEqual({ call: "config", agent_id: "vega" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(await screen.findByText("✓ Guardado en el hub")).toBeTruthy();
  });

  it("shows what the agent's kind does not allow, disabled and with its cause", async () => {
    render(<AgentSettings agents={AGENTS} defaultAgentId="atlas" agent={agent} />);
    await screen.findByText(
      "Atlas funciona con Hermes: su personalidad y lo que puede usar se cambian en su perfil de Hermes.",
    );
    for (const group of ["Pantalla", "Usar el Mac", "Calendario", "Reuniones y notas"]) {
      expect(screen.getByRole("switch", { name: group }).hasAttribute("disabled")).toBe(true);
    }
    expect(
      screen.getByText("No se puede cambiar desde Orbit: Hermes decide sus herramientas"),
    ).toBeTruthy();
    expect(
      screen.getByText("No se puede cambiar desde Orbit: La personalidad es el SOUL.md del perfil"),
    ).toBeTruthy();
    expect(
      (screen.getByRole("textbox", { name: "Personalidad de Atlas" }) as HTMLTextAreaElement)
        .readOnly,
    ).toBe(true);
    expect(screen.queryByRole("button", { name: "Guardar personalidad" })).toBeNull();
    // Skills not editable without a cause from the hub: said, not hidden.
    expect(screen.getByText("Este tipo de agente no deja cambiarlo desde Orbit.")).toBeTruthy();
    expect(screen.getByRole("switch", { name: "Activar agenda" }).hasAttribute("disabled")).toBe(
      true,
    );
    expect(screen.getByRole("button", { name: "Ver agenda" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Borrar agenda" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Nueva habilidad" })).toBeNull();
  });

  it("changes to another agent and reads its own configuration", async () => {
    await renderVega();
    fireEvent.click(screen.getByRole("button", { name: "Atlas" }));
    await screen.findByText(
      "Atlas funciona con Hermes: su personalidad y lo que puede usar se cambian en su perfil de Hermes.",
    );
    expect(calls.at(-1)).toEqual({ call: "config", agent_id: "atlas" });
  });

  it("saves the personality as edited", async () => {
    await renderVega();
    const save = screen.getByRole("button", { name: "Guardar personalidad" });
    expect(save.hasAttribute("disabled")).toBe(true);
    fireEvent.change(screen.getByRole("textbox", { name: "Personalidad de Vega" }), {
      target: { value: "# Vega\n\nSé muy breve." },
    });
    expect(screen.getByText("22 de 65.536 bytes")).toBeTruthy();
    fireEvent.click(save);
    await waitFor(() =>
      expect(calls[1]).toEqual({
        call: "personality",
        agent_id: "vega",
        text: "# Vega\n\nSé muy breve.",
      }),
    );
  });

  it("does not send a personality over the hub's ceiling and says both sizes", async () => {
    await renderVega();
    fireEvent.change(screen.getByRole("textbox", { name: "Personalidad de Vega" }), {
      target: { value: "é".repeat(32769) },
    });
    expect(screen.getByText("Ocupa 65.538 bytes; el máximo es 65.536.")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Guardar personalidad" }).hasAttribute("disabled"),
    ).toBe(true);
  });

  it("switches a skill on or off and deletes one after saying which", async () => {
    await renderVega();
    fireEvent.click(screen.getByRole("switch", { name: "Activar resumen-diario" }));
    await waitFor(() =>
      expect(calls[1]).toEqual({
        call: "skill_enabled",
        agent_id: "vega",
        name: "resumen-diario",
        enabled: false,
      }),
    );
    // The order is answered and the config read again before the next one.
    await screen.findByText("✓ Guardado en el hub");
    expect(calls).toHaveLength(3);
    fireEvent.click(screen.getByRole("button", { name: "Borrar resumen-diario" }));
    expect(calls).toHaveLength(3);
    fireEvent.click(screen.getByRole("button", { name: "Confirmar borrado" }));
    await waitFor(() =>
      expect(calls[3]).toEqual({ call: "skill_delete", agent_id: "vega", name: "resumen-diario" }),
    );
  });

  it("opens a skill's SKILL.md and saves it edited", async () => {
    await renderVega();
    fireEvent.click(screen.getByRole("button", { name: "Editar resumen-diario" }));
    const content = (await screen.findByRole("textbox", {
      name: "Contenido del SKILL.md",
    })) as HTMLTextAreaElement;
    expect(calls[1]).toEqual({ call: "skill", agent_id: "vega", name: "resumen-diario" });
    expect(content.value).toBe(SKILL_TEXT);
    fireEvent.change(content, { target: { value: `${SKILL_TEXT}\nUno más.` } });
    fireEvent.click(screen.getByRole("button", { name: "Guardar habilidad" }));
    await waitFor(() =>
      expect(calls[2]).toEqual({
        call: "skill_save",
        agent_id: "vega",
        name: "resumen-diario",
        content: `${SKILL_TEXT}\nUno más.`,
      }),
    );
  });

  it("creates a skill from a pasted SKILL.md, named by its frontmatter", async () => {
    await renderVega();
    fireEvent.click(screen.getByRole("button", { name: "Escribir desde cero" }));
    const content = screen.getByRole("textbox", { name: "Contenido del SKILL.md" });
    const save = screen.getByRole("button", { name: "Guardar habilidad" });
    expect(screen.getByText("Falta el name en el frontmatter del SKILL.md.")).toBeTruthy();
    expect(save.hasAttribute("disabled")).toBe(true);
    const pasted = "---\nname: notas-de-viaje\ndescription: Prepara un viaje.\n---\n\nPasos.";
    fireEvent.change(content, { target: { value: pasted } });
    expect(screen.getByText("Nombre: notas-de-viaje")).toBeTruthy();
    fireEvent.click(save);
    await waitFor(() =>
      expect(calls[1]).toEqual({
        call: "skill_save",
        agent_id: "vega",
        name: "notas-de-viaje",
        content: pasted,
      }),
    );
  });

  it("imports a folder: only its SKILL.md is read, and shown before it is saved", async () => {
    await renderVega();
    const text = "---\nname: notas-de-viaje\ndescription: Prepara un viaje.\n---\n\nPasos.";
    const files = [
      folderFile("notas-de-viaje/scripts/SKILL.md", "no es este"),
      folderFile("notas-de-viaje/SKILL.md", text),
      folderFile("notas-de-viaje/referencia.md", "otra cosa"),
    ];
    fireEvent.change(screen.getByLabelText("Carpeta con un SKILL.md"), { target: { files } });
    const content = (await screen.findByRole("textbox", {
      name: "Contenido del SKILL.md",
    })) as HTMLTextAreaElement;
    expect(content.value).toBe(text);
    fireEvent.click(screen.getByRole("button", { name: "Guardar habilidad" }));
    await waitFor(() =>
      expect(calls[1]).toEqual({
        call: "skill_save",
        agent_id: "vega",
        name: "notas-de-viaje",
        content: text,
      }),
    );
  });

  it("says a folder has no SKILL.md, and one too large for the hub, without reading it", async () => {
    await renderVega();
    const input = screen.getByLabelText("Carpeta con un SKILL.md");
    fireEvent.change(input, { target: { files: [folderFile("viaje/notas.md", "x")] } });
    expect(await screen.findByText("La carpeta «viaje» no tiene un SKILL.md.")).toBeTruthy();
    const large = folderFile("viaje/SKILL.md", "x".repeat(5000));
    const read = vi.spyOn(large, "text");
    fireEvent.change(input, { target: { files: [large] } });
    expect(
      await screen.findByText("El SKILL.md de «viaje» ocupa 5000 bytes; el máximo es 4096."),
    ).toBeTruthy();
    expect(read).not.toHaveBeenCalled();
    expect(calls).toHaveLength(1);
  });

  it("shows the hub's refusal with its numbers where the order was given", async () => {
    replies.set("skill_save", {
      outcome: "refused",
      refusal: { code: "too_many_skills", status: 409, numbers: { count: 8, max: 8 } },
    });
    await renderVega();
    fireEvent.click(screen.getByRole("button", { name: "Escribir desde cero" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Contenido del SKILL.md" }), {
      target: { value: "---\nname: otra\ndescription: Otra.\n---\n" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Guardar habilidad" }));
    expect((await screen.findByRole("alert")).textContent).toBe(
      "Este agente ya tiene 8 habilidades; el máximo es 8.",
    );
    // The editor stays open with what was written, to fix and send again.
    expect(screen.getByRole("textbox", { name: "Contenido del SKILL.md" })).toBeTruthy();
  });

  it("shows why the Mac could not ask the hub, and offers to read again", async () => {
    replies.set("config:vega", { outcome: "failed", refusal: { code: "not_paired" } });
    render(<AgentSettings agents={AGENTS} defaultAgentId="vega" agent={agent} />);
    expect((await screen.findByRole("alert")).textContent).toBe(
      "Este Mac no está emparejado con el hub. Emparéjalo en Ajustes.",
    );
    replies.delete("config:vega");
    fireEvent.click(screen.getByRole("button", { name: "Volver a leer" }));
    await screen.findByText("Vega se configura desde aquí. Los cambios se guardan en el hub.");
  });

  it("Ajustes shows the agents the status topic lists, from its default one", async () => {
    const snapshot = {
      connection: { daemon: "ready" },
      meeting: { call_preference: null },
      settings: {
        agents: AGENTS,
        default_agent_id: "atlas",
        permissions: {},
        hub: { paired: true, url: null },
        helper_connected: false,
        versions: { app: "test" },
        update: { state: "idle" },
        privacy_stopped: false,
      },
    } as unknown as AppView;
    render(
      <SettingsView
        agent={agent}
        error={null}
        onReopenOnboarding={() => undefined}
        send={() => undefined}
        snapshot={snapshot}
      />,
    );
    await screen.findByText(
      "Atlas funciona con Hermes: su personalidad y lo que puede usar se cambian en su perfil de Hermes.",
    );
    expect(calls[0]).toEqual({ call: "config", agent_id: "atlas" });
  });

  it("says no agent is known yet instead of an empty list", () => {
    render(<AgentSettings agents={[]} defaultAgentId={undefined} agent={agent} />);
    expect(
      screen.getByText("Todavía no se conocen los agentes del hub: aparecen al conectar con él."),
    ).toBeTruthy();
    expect(calls).toHaveLength(0);
  });
});

/** A file of a folder chosen in WebKit's panel: its path inside the folder,
 * and `text()`, which WebKit has and jsdom's `File` lacks. */
function folderFile(path: string, text: string): File {
  const file = new File([text], path.split("/").at(-1) ?? path, { type: "text/markdown" });
  Object.defineProperty(file, "webkitRelativePath", { value: path });
  Object.defineProperty(file, "text", {
    configurable: true,
    value: () => Promise.resolve(text),
    writable: true,
  });
  return file;
}
