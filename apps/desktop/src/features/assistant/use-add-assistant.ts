import { getCopy } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import { useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { AccessGroups } from "../../bridge/generated/AccessGroups";
import type { Agent } from "../../bridge/generated/Agent";
import type { AssistantColor } from "../../design/AssistantAvatar";

import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AgentPort } from "../settings/agents/use-agent-config";
import { callAgent } from "./agent-calls";
import { colorOf, profileProblem } from "./assistants";
import { agentsOf } from "./agents";
import { useSignIn } from "./use-sign-in";

export type Kind = "claude-code" | "codex" | "hermes";
export const KINDS: readonly Kind[] = ["claude-code", "codex", "hermes"];
export const STEPS = 4;

/** What a new assistant may use until its settings say otherwise (the annex's defaults). */
const DEFAULT_ACCESS: AccessGroups = {
  screen: false,
  control: false,
  calendar: true,
  meetings: true,
  files: true,
  notes: true,
};

/** The service's own name, made unique among the others: it is how it is called until renamed. */
export function provisionalName(kind: Kind, agents: readonly Agent[]): string {
  const base = getCopy().extras.assistantsCopy.kinds[kind]?.tag ?? kind;
  for (let index = 1; ; index += 1) {
    const name = index === 1 ? base : `${base} ${index}`;
    if (profileProblem(name, "", agents) === null) return name;
  }
}

/** The first colour nobody has, or violet when all four are taken. */
export function freeColor(agents: readonly Agent[]): AssistantColor {
  const used = new Set(agents.map((agent) => colorOf(agent, agents)));
  return ([1, 2, 3, 4] as const).find((color) => !used.has(color)) ?? 1;
}

/** The hub makes the assistant, signed out, under its service's own name until it is named. */
async function addProvisional(
  port: AgentPort | undefined,
  kind: Kind,
  color: AssistantColor,
  agents: readonly Agent[],
): Promise<{ agentId: string; name: string } | { refusal: string }> {
  const name = provisionalName(kind, agents);
  const result = await callAgent(port, {
    call: "add",
    service: kind,
    name,
    description: "",
    color: `c${color}`,
    access: DEFAULT_ACCESS,
  });
  if (!result.ok) return { refusal: result.text };
  if (result.outcome.outcome !== "added")
    return { refusal: getCopy().extras.assistantsCopy.notOffered };
  return { agentId: result.outcome.agent_id, name };
}

/** «Crear asistente»: its name and colour, then what it may use; the first refusal, or null. */
async function saveChoices(
  port: AgentPort | undefined,
  agentId: string,
  { name, color, access }: { name: string; color: AssistantColor; access: AccessGroups },
): Promise<string | null> {
  const profile = await callAgent(port, {
    call: "profile",
    agent_id: agentId,
    name: name.trim(),
    description: "",
    color: `c${color}`,
    primary: false,
  });
  if (!profile.ok) return profile.text;
  const groups = await callAgent(port, { call: "access", agent_id: agentId, groups: access });
  return groups.ok ? null : groups.text;
}

/** What the person chose so far: the service, the name, the colour and what it may use. */
function useChoices(agents: readonly Agent[]) {
  const [step, setStep] = useState(1);
  const [kind, setKind] = useState<Kind>("claude-code");
  const [name, setName] = useState("");
  const [color, setColor] = useState<AssistantColor>(() => freeColor(agents));
  const [access, setAccess] = useState<AccessGroups>(DEFAULT_ACCESS);
  return { step, setStep, kind, setKind, name, setName, color, setColor, access, setAccess };
}

/** The assistant the hub made for this window, the call under way and the last refusal. */
function useProgress() {
  const [created, setCreated] = useState<string | null>(null);
  const [busy, setBusy] = useState<"preparing" | "creating" | null>(null);
  const [refusal, setRefusal] = useState<string | null>(null);
  return { created, setCreated, busy, setBusy, refusal, setRefusal };
}

/**
 * «Añadir un asistente», step by step. The hub makes the assistant when the
 * sign-in begins (it needs one to sign in); leaving before «Crear asistente»
 * removes it again, so nothing half made stays among «Tus asistentes».
 */
type Send = (command: AppCommand) => unknown;

export function useAddAssistant(snapshot: AppSnapshot, port: AgentPort | undefined, send: Send) {
  const copy = useCopy().extras.assistantsCopy;
  const agents = agentsOf(snapshot);
  const choices = useChoices(agents);
  const { kind, name, color, access } = choices;
  const { created, setCreated, busy, setBusy, refusal, setRefusal } = useProgress();
  const mine = agents.find((agent) => agent.id === created);
  const vendor = getCopy().extras.vendorOf(kind) ?? copy.kinds[kind]?.tag ?? kind;
  const signIn = useSignIn({
    port,
    send,
    vendor,
    name: name === "" ? provisionalName(kind, agents) : name,
    signedIn: mine?.account === "signed_in",
  });
  const problem = profileProblem(
    name,
    created ?? "",
    agents.filter((agent) => agent.id !== created),
  );
  const discard = async () => {
    const id = created;
    if (id === null) return;
    setCreated(null);
    if (signIn.view.state === "waiting" || signIn.view.state === "starting") await signIn.cancel();
    await callAgent(port, { call: "remove", agent_id: id });
  };
  const make = async (): Promise<string | null> => {
    setBusy("preparing");
    const made = await addProvisional(port, kind, color, agents);
    setBusy(null);
    if ("refusal" in made) {
      setRefusal(made.refusal);
      return null;
    }
    setCreated(made.agentId);
    choices.setName(made.name);
    return made.agentId;
  };
  const begin = async () => {
    setRefusal(null);
    const id = created ?? (await make());
    if (id !== null) await signIn.start(id);
  };
  const create = async () => {
    if (created === null || problem !== null) return;
    setBusy("creating");
    setRefusal(null);
    const text = await saveChoices(port, created, { name, color, access });
    setBusy(null);
    if (text !== null) return setRefusal(text);
    choices.setStep(STEPS);
  };
  const connected = signIn.view.state === "done" || mine?.account === "signed_in";
  const state = { agents, created, mine, vendor, busy, refusal, setRefusal, problem };
  return { ...choices, ...state, signIn, connected, discard, begin, create };
}

export type AddFlow = ReturnType<typeof useAddAssistant>;
