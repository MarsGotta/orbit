import { useCopy } from "../../i18n/CopyProvider";
import { useCallback, useEffect, useState } from "react";
import type { AiSettings } from "../../bridge/generated/AiSettings";
import type { AgentCall } from "../../bridge/generated/AgentCall";

import type { AgentPort } from "./agents/use-agent-config";

export function useAISettings(agent?: AgentPort, load = true) {
  const copy = useCopy().extras.aiCopy;
  const aiRefusalText = useCopy().extras.aiRefusalText;
  const agentRefusalText = useCopy().extras.agentRefusalText;
  const agentFailureText = useCopy().extras.agentFailureText;
  const [settings, setSettings] = useState<AiSettings | null>(null);
  const [errorPlace, setErrorPlace] = useState("load");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState("");
  const call = useCallback(
    async (request: AgentCall) => {
      if (!agent) {
        setError(copy.unavailable);
        return false;
      }
      setErrorPlace(
        request.call === "ai_function_save"
          ? request.function
          : request.call === "ai_provider_save"
            ? "provider"
            : "load",
      );
      setBusy(true);
      setError("");
      setFeedback("");
      try {
        const { outcome } = await agent(request);
        if (outcome.outcome === "ai_settings") {
          setSettings(outcome.settings);
          if (request.call !== "ai_settings") setFeedback(copy.saved);
          return true;
        }
        setError(
          outcome.outcome === "refused"
            ? (aiRefusalText(outcome.refusal) ?? agentRefusalText(outcome.refusal))
            : outcome.outcome === "failed"
              ? agentFailureText(outcome.refusal)
              : copy.unavailable,
        );
      } catch {
        setError(copy.unavailable);
      } finally {
        setBusy(false);
      }
      return false;
    },
    [agent],
  );
  useEffect(() => {
    if (load) void call({ call: "ai_settings" });
  }, [call, load]);
  return { settings, error, errorPlace, busy, feedback, call };
}
