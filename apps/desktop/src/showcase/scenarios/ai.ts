import type { AiSettings } from "../../bridge/generated/AiSettings";
import { newMac } from "./first-run-macs";
import type { Scenario } from "../scenario";

const settings: AiSettings = {
  providers: [
    {
      id: "hub",
      name: "En tu hub",
      kind: "hub",
      base_url: null,
      key_configured: false,
      models: [{ id: "whisper", capabilities: ["transcription", "word_timestamps"] }],
    },
    {
      id: "cedar",
      name: "Cedro",
      kind: "local",
      base_url: "http://127.0.0.1:8080/v1",
      key_configured: false,
      models: [
        { id: "cedro-texto", capabilities: ["text", "tools"] },
        { id: "cedro-voz", capabilities: ["speech"] },
      ],
    },
    {
      id: "openrouter",
      name: "OpenRouter",
      kind: "openrouter",
      base_url: "https://openrouter.ai/api/v1",
      key_configured: true,
      models: [
        { id: "microsoft/mai-transcribe-2", capabilities: ["transcription", "word_timestamps"] },
      ],
    },
  ],
  functions: [
    {
      id: "meeting_transcription",
      provider_id: "openrouter",
      model: "microsoft/mai-transcribe-2",
      capability: "word_timestamps",
      privacy: "El audio sale a Microsoft a través de OpenRouter.",
    },
    {
      id: "listening",
      provider_id: "hub",
      model: "whisper",
      capability: "transcription",
      privacy: "El audio se queda en tu hub.",
    },
    {
      id: "speech",
      provider_id: "cedar",
      model: "cedro-voz",
      capability: "speech",
      privacy: "El texto se queda en tu hub.",
    },
    {
      id: "meeting_notes",
      provider_id: "operator",
      model: "modelo-notas",
      capability: "text",
      privacy: "El texto se queda en tu hub.",
    },
    {
      id: "vault",
      provider_id: "cedar",
      model: "cedro-texto",
      capability: "text",
      privacy: "El texto se queda en tu hub.",
    },
    {
      id: "assistants",
      provider_id: "cedar",
      model: "cedro-texto",
      capability: "tools",
      privacy: "El texto se queda en tu hub.",
    },
  ],
};

export const aiScenarios: Scenario[] = [
  {
    id: "ajustes-ia",
    view: "Ajustes",
    state: "IA",
    route: "#/settings/ia",
    description:
      "Providers and independent functions with privacy notices. Invented configuration only.",
    ready: "role=heading[name='IA']",
    agent: (call) =>
      call.call === "ai_settings" ? { outcome: { outcome: "ai_settings", settings } } : null,
  },
  {
    id: "ajustes-ia-funciones",
    view: "Ajustes",
    state: "Funciones de IA",
    route: "#/settings/ia",
    description: "Modelos por función y destino de los datos.",
    ready: "role=heading[name='Funciones']",
    agent: (call) =>
      call.call === "ai_settings" ? { outcome: { outcome: "ai_settings", settings } } : null,
    steps: [{ scroll: "role=heading[name='Funciones']" }],
  },
  {
    id: "bienvenida-ia",
    view: "Primer arranque",
    state: "Tu IA",
    route: "#/briefing",
    onboarded: false,
    views: (base) => newMac(base),
    description: "The hub-default AI onboarding step.",
    ready: "role=button[name='Empezar']",
    steps: [
      { click: "role=button[name='Empezar']" },
      { click: "role=button[name='Emparejar después']" },
      { click: "role=button[name='Continuar']" },
      { click: "role=button[name='Continuar']" },
    ],
  },
];
