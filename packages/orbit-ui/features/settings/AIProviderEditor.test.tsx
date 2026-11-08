import { fireEvent, render, screen, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import type { AgentCall } from "../../bridge/generated/AgentCall";
import type { AgentRefusal } from "../../bridge/generated/AgentRefusal";
import type { AiModel } from "../../bridge/generated/AiModel";
import type { AiProvider } from "../../bridge/generated/AiProvider";
import type { AiProviderInput } from "../../bridge/generated/AiProviderInput";
import { configureCopy, getCopy } from "../../i18n";
import { CopyProvider } from "../../i18n/CopyProvider";
import type { AgentPort } from "./agents/use-agent-config";
import { AISettings } from "./AISettings";

// An OpenAI-compatible API is never probed: the hub reads its catalogue and
// keeps the models the person names, with what each one does. These tests
// cover what the person sees and does in the provider editor, and exactly
// what the editor sends to the hub. Local AI and OpenRouter keep their forms.

const PRIVACY = "La nota se escribe con el modelo del operador.";
const hub: AiProvider = {
  id: "hub",
  name: "En tu hub",
  kind: "hub",
  base_url: null,
  key_configured: false,
  models: [{ id: "whisper", capabilities: ["transcription", "word_timestamps"] }],
};
const pinar: AiProvider = {
  id: "api-5d1e",
  name: "Pinar",
  kind: "api",
  base_url: "https://api.example.test/v1",
  key_configured: true,
  models: [
    { id: "pinar-oido", capabilities: ["transcription", "word_timestamps"] },
    { id: "pinar-pluma", capabilities: ["text"] },
  ],
};
const cedro: AiProvider = {
  id: "local-cedro",
  name: "Cedro",
  kind: "local",
  base_url: "http://127.0.0.1:8080/v1",
  key_configured: false,
  models: [{ id: "cedro-chico", capabilities: ["text"] }],
};

type Save = Extract<AgentCall, { call: "ai_provider_save" }>;

function port(providers: AiProvider[] = [], refusal?: AgentRefusal) {
  const settings = {
    providers: [hub, ...providers],
    functions: [
      {
        id: "meeting_notes",
        provider_id: "operator",
        model: "default",
        capability: "text",
        privacy: PRIVACY,
      },
    ],
  };
  return vi.fn<AgentPort>(async (call) => ({
    call,
    outcome:
      call.call === "ai_provider_save" && refusal !== undefined
        ? { outcome: "refused", refusal }
        : { outcome: "ai_settings", settings },
  }));
}

function lastSave(agent: ReturnType<typeof port>): Save | undefined {
  return agent.mock.calls
    .map(([call]) => call)
    .filter((call): call is Save => call.call === "ai_provider_save")
    .at(-1);
}

/** The provider as sent, with each model's capabilities in a stable order. */
function sent(save: Save | undefined): AiProviderInput | undefined {
  if (save === undefined) return undefined;
  const { models, ...rest } = save.provider;
  if (models === undefined) return rest;
  const sorted = (model: AiModel) => ({ ...model, capabilities: [...model.capabilities].sort() });
  return { ...rest, models: models.map(sorted) };
}

async function openNewProvider(agent: AgentPort) {
  render(<AISettings agent={agent} />);
  await screen.findByText(PRIVACY);
  fireEvent.click(screen.getByRole("button", { name: "Añadir un proveedor" }));
}

function chooseType(value: string) {
  const kind = screen.getByRole<HTMLSelectElement>("combobox", { name: "Tipo de proveedor" });
  fireEvent.change(kind, { target: { value } });
  expect(kind.value).toBe(value);
}

function type(label: string, value: string) {
  fireEvent.change(screen.getByLabelText(label), { target: { value } });
}

function nameModel(row: number, id: string, capabilities: string[] = []) {
  fireEvent.change(screen.getAllByLabelText("Modelo")[row]!, { target: { value: id } });
  for (const name of capabilities) fireEvent.click(screen.getAllByRole("checkbox", { name })[row]!);
}

function saveButton() {
  return screen.getByRole<HTMLButtonElement>("button", { name: "Comprobar y guardar" });
}

it.each([
  {
    language: "es",
    add: "Añadir un proveedor",
    providerType: "Tipo de proveedor",
    options: ["IA en local", "OpenRouter", "API compatible con OpenAI"],
    cloud: "OpenRouter, o cualquier API compatible con OpenAI con su URL y su clave.",
    model: "Modelo",
    addModel: "Añadir otro modelo",
    help: "Escribe cada modelo tal como lo lista la API y marca lo que hace. El hub comprueba que está en su catálogo, pero no lo prueba.",
    capabilities: ["Texto", "Herramientas", "Transcripción", "Tiempos por palabra", "Voz"],
  },
  {
    language: "en",
    add: "Add a provider",
    providerType: "Provider type",
    options: ["Local AI", "OpenRouter", "OpenAI-compatible API"],
    cloud: "OpenRouter, or any OpenAI-compatible API with its URL and key.",
    model: "Model",
    addModel: "Add another model",
    help: "Write each model as the API lists it and mark what it does. The hub checks it is in the catalogue but doesn’t try it.",
    capabilities: ["Text", "Tools", "Transcript", "Word timestamps", "Voice"],
  },
] as const)(
  "offers an OpenAI-compatible API with a list of named models in $language",
  async (c) => {
    render(
      <CopyProvider choice={c.language}>
        <AISettings agent={port()} />
      </CopyProvider>,
    );
    await screen.findByText(PRIVACY);
    fireEvent.click(screen.getByRole("button", { name: c.add }));
    const kind = screen.getByRole<HTMLSelectElement>("combobox", { name: c.providerType });
    expect(
      within(kind)
        .getAllByRole("option")
        .map((option) => option.textContent),
    ).toEqual(c.options);
    expect(kind.value).toBe("local");
    expect(screen.queryByLabelText(c.model)).toBeNull();
    fireEvent.change(kind, { target: { value: "api" } });
    expect(screen.getAllByLabelText(c.model)).toHaveLength(1);
    expect(screen.getAllByRole("checkbox")).toHaveLength(5);
    for (const name of c.capabilities)
      expect(screen.getByRole<HTMLInputElement>("checkbox", { name }).checked).toBe(false);
    expect(screen.getByText(c.help)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: c.addModel }));
    expect(screen.getAllByLabelText(c.model)).toHaveLength(2);
    expect(screen.getByText(c.cloud)).toBeTruthy();
  },
);

it("saves an API with its URL, its key and the models the person names, each with what it does", async () => {
  const agent = port();
  await openNewProvider(agent);
  chooseType("api");
  type("Nombre del proveedor", "Pinar");
  type("URL base", "https://api.example.test/v1");
  type("Clave (opcional)", "invented-api-key");
  nameModel(0, "pinar-oido", ["Transcripción", "Tiempos por palabra"]);
  fireEvent.click(screen.getByRole("button", { name: "Añadir otro modelo" }));
  nameModel(1, "pinar-pluma", ["Texto", "Herramientas"]);
  fireEvent.click(saveButton());
  expect(await screen.findByText("Guardado en el hub")).toBeTruthy();
  const save = lastSave(agent);
  expect(save?.provider_id).toMatch(/^[a-z0-9-]{1,64}$/);
  expect(["hub", "operator", "openrouter"]).not.toContain(save?.provider_id);
  expect(sent(save)).toEqual({
    name: "Pinar",
    kind: "api",
    base_url: "https://api.example.test/v1",
    api_key: "invented-api-key",
    models: [
      { id: "pinar-oido", capabilities: ["transcription", "word_timestamps"] },
      { id: "pinar-pluma", capabilities: ["text", "tools"] },
    ],
  });
  expect(screen.queryByDisplayValue("invented-api-key")).toBeNull();
});

it("saves an API only with a name, a URL and a named model, and sends no blank model", async () => {
  const agent = port();
  await openNewProvider(agent);
  chooseType("api");
  expect(saveButton().disabled).toBe(true);
  type("Nombre del proveedor", "Pinar");
  type("URL base", "https://api.example.test/v1");
  expect(saveButton().disabled).toBe(true);
  nameModel(0, "   ");
  expect(saveButton().disabled).toBe(true);
  nameModel(0, "  pinar-pluma  ", ["Texto"]);
  expect(saveButton().disabled).toBe(false);
  type("URL base", "");
  expect(saveButton().disabled).toBe(true);
  type("URL base", "https://api.example.test/v1");
  fireEvent.click(screen.getByRole("button", { name: "Añadir otro modelo" }));
  expect(saveButton().disabled).toBe(false);
  fireEvent.click(saveButton());
  expect(await screen.findByText("Guardado en el hub")).toBeTruthy();
  expect(sent(lastSave(agent))?.models).toEqual([{ id: "pinar-pluma", capabilities: ["text"] }]);
});

it("edits an API with its models and what each does, and saves them again keeping the key", async () => {
  const agent = port([pinar]);
  render(<AISettings agent={agent} />);
  fireEvent.click(await screen.findByRole("button", { name: "Editar Pinar" }));
  const kind = screen.getByRole<HTMLSelectElement>("combobox", { name: "Tipo de proveedor" });
  expect(kind.value).toBe("api");
  expect(kind.disabled).toBe(true);
  expect(screen.getByLabelText<HTMLInputElement>("Nombre del proveedor").value).toBe("Pinar");
  expect(screen.getByLabelText<HTMLInputElement>("URL base").value).toBe(
    "https://api.example.test/v1",
  );
  const models = screen.getAllByLabelText<HTMLInputElement>("Modelo");
  expect(models.map((field) => field.value)).toEqual(["pinar-oido", "pinar-pluma"]);
  const checked = (name: string) =>
    screen.getAllByRole<HTMLInputElement>("checkbox", { name }).map((box) => box.checked);
  expect(checked("Transcripción")).toEqual([true, false]);
  expect(checked("Tiempos por palabra")).toEqual([true, false]);
  expect(checked("Texto")).toEqual([false, true]);
  expect(checked("Herramientas")).toEqual([false, false]);
  fireEvent.click(screen.getAllByRole("checkbox", { name: "Herramientas" })[1]!);
  fireEvent.click(saveButton());
  expect(await screen.findByText("Guardado en el hub")).toBeTruthy();
  const save = lastSave(agent);
  expect(save?.provider_id).toBe("api-5d1e");
  expect(sent(save)).toEqual({
    name: "Pinar",
    kind: "api",
    base_url: "https://api.example.test/v1",
    models: [
      { id: "pinar-oido", capabilities: ["transcription", "word_timestamps"] },
      { id: "pinar-pluma", capabilities: ["text", "tools"] },
    ],
  });
});

it("does not offer to correct detected capabilities for an API; a local server keeps it", async () => {
  render(<AISettings agent={port([pinar, cedro])} />);
  await screen.findByText("Pinar");
  expect(screen.getAllByText("Corregir capacidades detectadas")).toHaveLength(1);
  expect(screen.getByText("cedro-chico", { selector: "legend" })).toBeTruthy();
  expect(screen.queryByText("pinar-oido", { selector: "legend" })).toBeNull();
});

it("says at the form how many named models the API does not list", async () => {
  const agent = port([], {
    code: "ai_models_invalid",
    status: 400,
    numbers: { count: 1, named: 2, catalogue: 34 },
  });
  await openNewProvider(agent);
  chooseType("api");
  type("Nombre del proveedor", "Pinar");
  type("URL base", "https://api.example.test/v1");
  type("Clave (opcional)", "synthetic-api-key");
  nameModel(0, "pinar-oido", ["Transcripción"]);
  fireEvent.click(screen.getByRole("button", { name: "Añadir otro modelo" }));
  nameModel(1, "pinar-plumaa", ["Texto"]);
  fireEvent.click(saveButton());
  expect((await screen.findByRole("alert")).textContent).toBe(
    "Modelos que no están en el catálogo de esta API: 1 de 2. El catálogo tiene 34; escríbelos tal como los lista.",
  );
  expect(screen.queryByDisplayValue("synthetic-api-key")).toBeNull();
  expect(screen.getAllByLabelText<HTMLInputElement>("Modelo").map((f) => f.value)).toEqual([
    "pinar-oido",
    "pinar-plumaa",
  ]);
  expect(saveButton().disabled).toBe(false);
});

it.each([
  {
    language: "es",
    locale: "es-ES",
    counted:
      "Modelos que no están en el catálogo de esta API: 1 de 2. El catálogo tiene 34; escríbelos tal como los lista.",
    plain: "Corrige sólo los modelos que devolvió el proveedor.",
  },
  {
    language: "en",
    locale: "en-US",
    counted:
      "Models not in this API’s catalogue: 1 of 2. The catalogue has 34; write them as it lists them.",
    plain: "Only change models returned by the provider.",
  },
] as const)(
  "names an API's unlisted models with their numbers, and keeps the correction refusal, in $language",
  ({ language, locale, counted, plain }) => {
    configureCopy(language, [locale], "Atlas");
    const text = getCopy().extras.aiRefusalText;
    expect(text({ code: "ai_models_invalid", status: 400, numbers: {} })).toBe(plain);
    expect(
      text({
        code: "ai_models_invalid",
        status: 400,
        numbers: { count: 1, named: 2, catalogue: 34 },
      }),
    ).toBe(counted);
  },
);

it("local AI still sends its name, URL and key, and no models", async () => {
  const agent = port();
  await openNewProvider(agent);
  expect(screen.queryByLabelText("Modelo")).toBeNull();
  type("Nombre del proveedor", "Cedro");
  type("URL base", "http://127.0.0.1:8080/v1");
  type("Clave (opcional)", "invented-key");
  fireEvent.click(saveButton());
  expect(await screen.findByText("Guardado en el hub")).toBeTruthy();
  const save = lastSave(agent);
  expect(save?.provider_id.startsWith("local-")).toBe(true);
  expect(save?.provider).toEqual({
    name: "Cedro",
    kind: "local",
    base_url: "http://127.0.0.1:8080/v1",
    api_key: "invented-key",
  });
});

it("OpenRouter still sends its own name and URL with the key, and no models", async () => {
  const agent = port();
  await openNewProvider(agent);
  chooseType("openrouter");
  expect(screen.queryByLabelText("Nombre del proveedor")).toBeNull();
  expect(screen.queryByLabelText("URL base")).toBeNull();
  expect(screen.queryByLabelText("Modelo")).toBeNull();
  type("Clave (opcional)", "invented-router-key");
  fireEvent.click(saveButton());
  expect(await screen.findByText("Guardado en el hub")).toBeTruthy();
  const save = lastSave(agent);
  expect(save?.provider_id).toBe("openrouter");
  expect(save?.provider).toEqual({
    name: "OpenRouter",
    kind: "openrouter",
    base_url: "https://openrouter.ai/api/v1",
    api_key: "invented-router-key",
  });
});

it("sends no models for local AI after the person tried the API type first", async () => {
  const agent = port();
  await openNewProvider(agent);
  chooseType("api");
  nameModel(0, "pinar-pluma", ["Texto"]);
  chooseType("local");
  expect(screen.queryByLabelText("Modelo")).toBeNull();
  type("Nombre del proveedor", "Cedro");
  type("URL base", "http://127.0.0.1:8080/v1");
  fireEvent.click(saveButton());
  expect(await screen.findByText("Guardado en el hub")).toBeTruthy();
  expect(lastSave(agent)?.provider).toEqual({
    name: "Cedro",
    kind: "local",
    base_url: "http://127.0.0.1:8080/v1",
  });
});
