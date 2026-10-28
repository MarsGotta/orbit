import { formatQuantity } from "../locale";
import { numberFormat } from "../locale";
import { transcriptionRefusals } from "./transcription";
import type { AccessGroups } from "../../bridge/generated/AccessGroups";
import type { AgentRefusal } from "../../bridge/generated/AgentRefusal";
import type { RefusalNumber } from "../../bridge/generated/RefusalNumber";
import { assistantRefusals } from "./assistantRefusals";
import { count, text, type Numbers } from "./refusalNumbers";

/** A count as the person reads it: «16.384». */
const figure = numberFormat().format;

/**
 * The groups of Orbit's tools an agent may use, in the order Ajustes shows
 * them (hub-v1 «Agents»), each said by what it lets the agent do. The tools
 * each one opens are the hub's (`TOOLS` in `server/orbit_hub/assistant/access.py`).
 */
export const accessGroups: ReadonlyArray<{
  group: keyof AccessGroups;
  label: string;
  description: string;
}> = [
  {
    group: "screen",
    label: "Screen",
    description: "View your windows, take screenshots and read selected text",
  },
  { group: "control", label: "Use the Mac", description: "Click, type and open apps for you" },
  { group: "calendar", label: "Calendar", description: "Read your agenda" },
  {
    group: "meetings",
    label: "Meetings",
    description: "Read your meetings, notes and transcripts",
  },
  { group: "files", label: "Files", description: "Read folders you choose below" },
  { group: "notes", label: "Notes", description: "Search and read vault notes on the hub" },
];

/** Every number as `key: value`, for a refusal this screen has no words for. */
function listed(numbers: Numbers): string {
  return Object.entries(numbers)
    .filter((entry): entry is [string, RefusalNumber] => entry[1] !== undefined)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, value]) => `${key}: ${String(value)}`)
    .join(", ");
}

/** The words of each refusal; `undefined` when a number it needs is missing. */
const refusals: Record<string, (numbers: Numbers) => string | undefined> = {
  ...Object.fromEntries(
    Object.entries(transcriptionRefusals).map(([code, value]) => [code, () => value]),
  ),
  invalid_working_folder: (n) => {
    const cause = text(n, "cause");
    return cause === undefined ? undefined : `The working folder ${cause}`;
  },
  agent_not_found: (n) => {
    const agent = text(n, "agent_id");
    if (agent === undefined) return undefined;
    const known = text(n, "agent_ids");
    return `The hub has no agent “${agent}”${known ? `; it has ${known}` : ""}.`;
  },
  skill_delete_unsupported: () =>
    "Hermes can’t delete skills from this panel. Disable it here or delete it on the Hermes server.",
  skill_invalid: (n) => {
    const name = text(n, "name");
    const rule = text(n, "rule");
    if (name === undefined || rule === undefined) return undefined;
    return `Skill “${name}” is invalid: it doesn’t satisfy rule “${rule}”.`;
  },
  skill_not_found: (n) => {
    const name = text(n, "name");
    return name === undefined ? undefined : `The hub has no skill “${name}”.`;
  },
  skill_too_large: (n) => {
    const name = text(n, "name");
    const bytes = count(n, "bytes");
    const max = count(n, "max_bytes");
    if (name === undefined || bytes === undefined || max === undefined) return undefined;
    return `Skill “${name}” takes ${figure(bytes)} bytes; the maximum is ${figure(max)}.`;
  },
  too_many_skills: (n) => {
    const skills = count(n, "count");
    const max = count(n, "max");
    if (skills === undefined || max === undefined) return undefined;
    return `This agent already has ${numberFormat().format(skills)} skills; the maximum is ${numberFormat().format(max)}.`;
  },
  personality_too_large: (n) => {
    const bytes = count(n, "bytes");
    const max = count(n, "max_bytes");
    if (bytes === undefined || max === undefined) return undefined;
    return `The personality takes ${figure(bytes)} bytes; the maximum is ${figure(max)}.`;
  },
  ...assistantRefusals,
  assistant_admin_refused: (n) => {
    const route = text(n, "route");
    const status = count(n, "status");
    if (route === undefined || status === undefined) return undefined;
    return `Hermes rejected the change: ${route} returned HTTP ${numberFormat().format(status)}.`;
  },
};

/** The hub's refusal of a read or an order, with its cause and numbers. */
export function agentRefusalText(refusal: AgentRefusal): string {
  const words = Object.hasOwn(refusals, refusal.code)
    ? refusals[refusal.code]?.(refusal.numbers)
    : undefined;
  if (words !== undefined) return words;
  const numbers = listed(refusal.numbers);
  return `The hub rejected the command (${refusal.code}, HTTP ${numberFormat().format(refusal.status)})${numbers ? `: ${numbers}` : ""}.`;
}

/**
 * Why the Mac could not ask the hub, or could not read its answer: the
 * daemon's typed refusal (`Refusal`), with its numbers.
 */
export function agentFailureText(refusal: Record<string, unknown>): string {
  const status = typeof refusal.status === "number" ? refusal.status : null;
  switch (refusal.code) {
    case "not_paired":
      return "This Mac isn’t paired with the hub. Pair it in Settings.";
    case "privacy_stopped":
      return "Everything is stopped. Rearm Orbit to read or change agents.";
    case "hub_unreachable":
      return "No connection to the hub, where agents are read and changed.";
    case "upstream_failed":
      if (status === 404)
        return "The hub returned HTTP 404 without a code. Its version may not support agent settings yet.";
      return status === null
        ? "The hub didn’t return a response: the connection dropped or timed out. Read again to check whether the change was applied."
        : `The hub returned HTTP ${numberFormat().format(status)}.`;
    case "hub_reply_invalid":
      return `The hub response doesn’t match the contract (field ${String(refusal.field)}).`;
    case "too_large":
      return `The hub response takes ${formatQuantity(refusal.bytes)} bytes; the maximum is ${formatQuantity(refusal.max_bytes)}.`;
    default:
      return `Orbit couldn’t query the hub (${String(refusal.code)}).`;
  }
}

export const agents = {
  agents: {
    none: "The hub’s agents aren’t known yet. They’ll appear when connected.",
    unavailable: "This window can’t read agents.",
    loading: (name: string) => `Reading settings for ${name}…`,
    readAgain: "Read again",
    notEditable: (cause: string | null | undefined) =>
      cause
        ? `Can’t change this from Orbit: ${cause}`
        : "This agent type can’t be changed from Orbit.",
    saving: "Saving to the hub…",
    saved: "✓ Saved to the hub",
    bytes: (count: number, max: number) => `${figure(count)} of ${figure(max)} bytes`,
    overLimit: (count: number, max: number) =>
      `Takes ${figure(count)} bytes; the maximum is ${figure(max)}.`,
    personality: "Personality",
    personalityLabel: (name: string) => `Personality for ${name}`,
    savePersonality: "Save",
    workingFolder: "Working folder",
    workingFolderLabel: "Working folder on the hub",
    workingFolderHelp:
      "Absolute server path where it starts working. Changes apply when a conversation opens.",
    saveWorkingFolder: "Save folder",
    access: "What it can use",
    accessHelp: "macOS still controls access to this Mac. These settings only limit this agent.",
    skills: "Skills",
    skillCount: (count: number, max: number) =>
      `${numberFormat().format(count)} of ${numberFormat().format(max)}`,
    noSkills:
      "This agent has no skills yet. A skill teaches a specific task, such as preparing for a meeting.",
    enable: (name: string) => `Enable ${name}`,
    view: (name: string) => `View ${name}`,
    edit: (name: string) => `Edit ${name}`,
    viewLabel: "View",
    editLabel: "Edit",
    delete: (name: string) => `Delete ${name}`,
    deleteLabel: "Delete",
    deleteQuestion: (name: string) => `Delete ${name} from this agent? This can’t be undone.`,
    confirmDelete: "Yes, delete",
    cancel: "Cancel",
    newSkill: "New skill",
    newSkillMenu: "How to start the skill",
    fromScratch: "Write from scratch",
    fromScratchHelp: "A template with a name and description to fill in.",
    importFolder: "Import folder",
    importFolderHelp: "A folder with a SKILL.md. Only that file is read and sent.",
    folderInput: "Folder containing a SKILL.md",
    noSkillFile: (folder: string) => `Folder “${folder}” has no SKILL.md.`,
    skillFileTooLarge: (folder: string, size: number, max: number) =>
      `The SKILL.md in “${folder}” takes ${figure(size)} bytes; the maximum is ${figure(max)}.`,
    skillFileUnreadable: (folder: string) => `Couldn’t read the SKILL.md in “${folder}”.`,
    editorNew: "New skill",
    editorHelp:
      "The complete SKILL.md. Its name and description go at the top, between the two lines of dashes.",
    content: "SKILL.md content",
    name: (name: string) => `Will be called ${name}`,
    noName: "The SKILL.md frontmatter is missing name.",
    saveSkill: "Save skill",
    close: "Close",
    template: "---\nname: \ndescription: \n---\n\n",
  },
} as const;
