import { useEffect, useState } from "react";
import type { TranscriptionModel } from "../../bridge/generated/TranscriptionModel";
import type { TranscriptionSettings as Settings } from "../../bridge/generated/TranscriptionSettings";
import { agentFailureText, agentRefusalText } from "../../i18n/es/agents";
import { transcription as copy } from "../../i18n/es/transcription";
import type { AgentPort } from "./agents/use-agent-config";

export function useTranscriptionSettings({ agent }: { agent?: AgentPort }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [model, setModel] = useState<TranscriptionModel>("whisper");
  const [key, setKey] = useState("");
  const [feedback, setFeedback] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let alive = true;
    if (agent)
      void agent({ call: "transcription_settings" })
        .then(({ outcome }) => {
          if (!alive) return;
          if (outcome.outcome === "transcription_settings") {
            setSettings(outcome.settings);
            setModel(outcome.settings.model);
          } else if (outcome.outcome === "refused") setError(agentRefusalText(outcome.refusal));
          else if (outcome.outcome === "failed") setError(agentFailureText(outcome.refusal));
          else setError(copy.invalid);
        })
        .catch(() => {
          if (alive) setError(copy.unavailable);
        });
    return () => {
      alive = false;
    };
  }, [agent]);
  const save = async () => {
    if (!agent) return;
    setBusy(true);
    setError("");
    setFeedback("");
    try {
      const { outcome } = await agent({
        call: "transcription_save",
        model,
        ...(key ? { api_key: key } : {}),
      });
      if (outcome.outcome === "transcription_settings") {
        setSettings(outcome.settings);
        setModel(outcome.settings.model);
        setKey("");
        setFeedback(copy.saved);
      } else if (outcome.outcome === "refused") setError(agentRefusalText(outcome.refusal));
      else if (outcome.outcome === "failed") setError(agentFailureText(outcome.refusal));
      else setError(copy.invalid);
    } catch {
      setError(copy.unavailable);
    } finally {
      setBusy(false);
    }
  };
  return { settings, model, setModel, key, setKey, feedback, setFeedback, error, busy, save };
}
