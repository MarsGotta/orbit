import { useEffect, useRef, useState } from "react";
import type { Distillation } from "../../bridge/generated/Distillation";
import type { VaultCall } from "../../bridge/generated/VaultCall";
import { message, request, type VaultPort } from "./api";
type Activity = "review" | "apply" | "discard" | "undo" | "prepare" | null;

function useProposal(port: VaultPort | undefined, id: string) {
  const [proposal, setProposal] = useState<Distillation | null>(null);
  const [failure, setFailure] = useState<unknown>(null);
  const [attempt, setAttempt] = useState(0);
  const version = useRef(0);
  useEffect(() => {
    let alive = true;
    let timer: number;
    const generation = ++version.current;
    setProposal(null);
    setFailure(null);
    const read = async () => {
      try {
        const answer = await request(port, { call: "distillation", id });
        if (!alive || version.current !== generation || answer.outcome !== "distillation") return;
        setProposal(answer.proposal);
        if (["queued", "preparing", "applying", "undoing"].includes(answer.proposal.status))
          timer = window.setTimeout(() => void read(), 2000);
      } catch (error: unknown) {
        if (alive && version.current === generation) setFailure(error);
      }
    };
    void read();
    return () => {
      alive = false;
      window.clearTimeout(timer);
      version.current++;
    };
  }, [id, port, attempt]);
  return {
    proposal,
    setProposal,
    failure,
    setFailure,
    version,
    reload: () => setAttempt((n) => n + 1),
  };
}

async function execute(port: VaultPort | undefined, id: string, call: VaultCall) {
  try {
    const answer = await request(port, call);
    return {
      proposal: answer.outcome === "distillation" ? answer.proposal : null,
      error: null,
      success: true,
    };
  } catch (error: unknown) {
    // Reconcile a lost receipt. Never repeat a write merely because it failed.
    try {
      const answer = await request(port, { call: "distillation", id });
      return {
        proposal: answer.outcome === "distillation" ? answer.proposal : null,
        error,
        success: false,
      };
    } catch {
      return { proposal: null, error, success: false };
    }
  }
}

export function useDistillation(port: VaultPort | undefined, id: string) {
  const reader = useProposal(port, id);
  const [activity, setActivity] = useState<Activity>(null);
  const inFlight = useRef(false);
  const mutate = async (call: VaultCall) => {
    if (inFlight.current) return false;
    inFlight.current = true;
    const generation = ++reader.version.current;
    setActivity(call.call === "distillation_action" ? call.action : "review");
    reader.setFailure(null);
    const result = await execute(port, id, call);
    if (reader.version.current !== generation) {
      inFlight.current = false;
      return false;
    }
    reader.setFailure(result.error);
    if (result.proposal !== null) {
      reader.setProposal(result.proposal);
      if (result.success && result.proposal.id !== id)
        window.location.hash = `#/vault/distill/${encodeURIComponent(result.proposal.id)}`;
      window.dispatchEvent(new Event("orbit-vault-changed"));
      if (
        call.call === "distillation_action" &&
        (call.action === "apply" || call.action === "undo")
      )
        window.dispatchEvent(new Event("orbit-vault-content-changed"));
      if (
        result.proposal.id === id &&
        ["queued", "preparing", "applying", "undoing"].includes(result.proposal.status)
      )
        reader.reload();
    }
    inFlight.current = false;
    setActivity(null);
    return result.success;
  };
  return {
    proposal: reader.proposal,
    activity,
    busy: activity !== null,
    mutate,
    error: reader.failure === null ? null : message(reader.failure),
    reload: reader.reload,
  };
}
