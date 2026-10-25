import type { AgentConfig } from "../../bridge/generated/AgentConfig";
import type { SkillDocument } from "../../bridge/generated/SkillDocument";

// The hub's ceilings: a personality of 16 KiB, 128 skills of 64 KiB each
// (`orbit_hub/assistant/agent_config.py`, `skills.py`).

const deimos: AgentConfig = {
  agent_id: "deimos",
  kind: "hermes",
  personality: {
    text: "Soy Deimos, el asistente de Lucía. Hablo claro y breve, en español, y antes de tocar nada que no se pueda deshacer pregunto.",
    editable: false,
    max_bytes: 16_384,
    cause: "La personalidad es el SOUL.md del perfil de Hermes",
  },
  access: {
    groups: { screen: true, control: true, calendar: true, meetings: true, files: true },
    editable: false,
    cause: "Hermes decide sus herramientas",
  },
  skills: {
    editable: false,
    cause: "Las habilidades de Hermes se cambian en su perfil",
    max_skills: 128,
    max_skill_bytes: 65_536,
    items: [
      {
        name: "preparar-reunion",
        description: "Repasa las notas anteriores de una reunión.",
        enabled: true,
      },
      {
        name: "correo-cliente",
        description: "Redacta correos a clientes con el tono de Nubaria.",
        enabled: true,
      },
      {
        name: "informe-semanal",
        description: "Resume la semana de reuniones los viernes.",
        enabled: false,
      },
    ],
  },
};

const claudeCode: AgentConfig = {
  agent_id: "claude-code",
  kind: "codex",
  personality: {
    text: "Trabajas en el repositorio de reservas de Nubaria. Explica cada cambio antes de hacerlo y ejecuta los tests al terminar.",
    editable: true,
    max_bytes: 16_384,
    cause: null,
  },
  access: {
    groups: { screen: false, control: false, calendar: false, meetings: true, files: true },
    editable: true,
    cause: null,
  },
  skills: {
    editable: true,
    cause: null,
    max_skills: 128,
    max_skill_bytes: 65_536,
    items: [
      {
        name: "revisar-pr",
        description: "Revisa una pull request con la guía del equipo.",
        enabled: true,
      },
      {
        name: "migraciones",
        description: "Escribe migraciones de Postgres reversibles.",
        enabled: true,
      },
    ],
  },
};

const codex: AgentConfig = {
  ...claudeCode,
  agent_id: "codex",
  personality: {
    ...claudeCode.personality,
    text: "Eres conciso. Propón el cambio más pequeño que funcione.",
  },
  skills: { ...claudeCode.skills, items: [] },
};

export const agentConfigs: Record<string, AgentConfig> = {
  deimos,
  "claude-code": claudeCode,
  codex,
};

export const skillDocuments: Record<string, SkillDocument> = {
  "revisar-pr": {
    name: "revisar-pr",
    description: "Revisa una pull request con la guía del equipo.",
    enabled: true,
    content:
      "---\nname: revisar-pr\ndescription: Revisa una pull request con la guía del equipo.\n---\n\n1. Lee la descripción y los tests.\n2. Comprueba que cada migración se puede deshacer.\n3. Señala lo que falte, sin reescribir el código.",
  },
};
