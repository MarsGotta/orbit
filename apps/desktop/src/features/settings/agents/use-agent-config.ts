import { useCallback, useEffect, useRef, useState } from "react";
import type { AgentAnswer } from "../../../bridge/generated/AgentAnswer";
import type { AgentCall } from "../../../bridge/generated/AgentCall";
import type { AgentConfig } from "../../../bridge/generated/AgentConfig";
import type { SkillDocument } from "../../../bridge/generated/SkillDocument";
import { agentFailureText, agentRefusalText } from "../../../i18n/es/agents";

/** The daemon's one call for an agent's configuration (`orbit_agent`). */
export type AgentPort = (call: AgentCall) => Promise<AgentAnswer>;

/** Where an order was given, so its outcome is said there. */
export type Place = "personality" | "access" | "skills";

export type Feedback =
  { place: Place; kind: "saving" | "saved" } | { place: Place; kind: "error"; text: string };

export type ConfigView =
  | { state: "loading" }
  | { state: "config"; config: AgentConfig }
  | { state: "failed"; text: string };

type Result =
  | { kind: "config"; config: AgentConfig }
  | { kind: "skill"; skill: SkillDocument }
  | { kind: "done" }
  | { kind: "error"; text: string };

function thrownText(error: unknown): string {
  if (typeof error === "object" && error !== null && "message" in error)
    return String(error.message);
  return String(error);
}

async function ask(agent: AgentPort, call: AgentCall): Promise<Result> {
  try {
    const { outcome } = await agent(call);
    switch (outcome.outcome) {
      case "config":
        return { kind: "config", config: outcome.config };
      case "skill":
        return { kind: "skill", skill: outcome.skill };
      // `added` and `sign_in` answer the assistant calls, which this screen does not make.
      case "done":
      case "added":
      case "sign_in":
        return { kind: "done" };
      case "refused":
        return { kind: "error", text: agentRefusalText(outcome.refusal) };
      case "failed":
        return { kind: "error", text: agentFailureText(outcome.refusal) };
    }
  } catch (error: unknown) {
    return { kind: "error", text: thrownText(error) };
  }
}

/**
 * What a read of the configuration shows. Any answer other than the
 * configuration or its refusal is not it: a failed read, never «Leyendo…»
 * for ever.
 */
function configView(result: Result): ConfigView {
  if (result.kind === "config") return { state: "config", config: result.config };
  if (result.kind === "error") return { state: "failed", text: result.text };
  return {
    state: "failed",
    text: agentFailureText({ code: "hub_reply_invalid", field: "outcome" }),
  };
}

/** Whether the screen that asked is still there when the hub answers. */
function useAlive() {
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  return alive;
}

/**
 * One agent's configuration as the hub has it, read when the screen opens
 * and again after every order the hub takes. Nothing is kept beyond what the
 * screen shows: the hub is the single source.
 */
export function useAgentConfig(agent: AgentPort, agentId: string) {
  const [view, setView] = useState<ConfigView>({ state: "loading" });
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const [busy, setBusy] = useState(false);
  const alive = useAlive();

  const read = useCallback(async () => {
    const result = await ask(agent, { call: "config", agent_id: agentId });
    if (!alive.current) return;
    setView(configView(result));
  }, [agent, agentId, alive]);

  useEffect(() => {
    void read();
  }, [read]);

  const reload = useCallback(() => {
    setView({ state: "loading" });
    void read();
  }, [read]);

  /** One order; `true` when the hub took it. The config is read again then. */
  const order = useCallback(
    async (call: AgentCall, place: Place): Promise<boolean> => {
      setBusy(true);
      setFeedback({ place, kind: "saving" });
      const result = await ask(agent, call);
      if (result.kind === "done") await read();
      if (!alive.current) return false;
      setFeedback(
        result.kind === "error"
          ? { place, kind: "error", text: result.text }
          : { place, kind: "saved" },
      );
      setBusy(false);
      return result.kind === "done";
    },
    [agent, alive, read],
  );

  /** The whole `SKILL.md` of one skill; its refusal said among the skills. */
  const skill = useCallback(
    async (name: string): Promise<SkillDocument | null> => {
      setBusy(true);
      setFeedback(null);
      const result = await ask(agent, { call: "skill", agent_id: agentId, name });
      if (!alive.current) return null;
      setBusy(false);
      if (result.kind === "skill") return result.skill;
      setFeedback({
        place: "skills",
        kind: "error",
        text:
          result.kind === "error"
            ? result.text
            : agentFailureText({ code: "hub_reply_invalid", field: "outcome" }),
      });
      return null;
    },
    [agent, agentId, alive],
  );

  /** A cause found on the Mac (a folder without a `SKILL.md`), said there. */
  const say = useCallback((place: Place, text: string) => {
    setFeedback({ place, kind: "error", text });
  }, []);

  return { view, feedback, busy, reload, order, skill, say };
}
