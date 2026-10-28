import { useCopy } from "../../i18n/CopyProvider";
import { useState } from "react";
import type { AgentPort } from "../settings/agents/use-agent-config";
import { AIProviderEditor } from "../settings/AIProviderEditor";
import { useAISettings } from "../settings/use-ai-settings";
import { Step } from "./OnboardingSteps";

export function AIStep({ agent }: { agent?: AgentPort }) {
  const inlineCopy = useCopy().onboardingInline;
  const [add, setAdd] = useState(false);
  const view = useAISettings(agent, false);
  return (
    <Step title={inlineCopy.yourAI} lead={inlineCopy.lead}>
      <label>
        <input type="radio" name="ai-setup" checked={!add} onChange={() => setAdd(false)} />
        {inlineCopy.allInHub}
      </label>
      <p>{inlineCopy.providerHelp}</p>
      <label>
        <input type="radio" name="ai-setup" checked={add} onChange={() => setAdd(true)} />
        {inlineCopy.addProvider}
      </label>
      {add && <AIProviderEditor view={view} onClose={() => setAdd(false)} />}
      {view.feedback.length > 0 && <p role="status">{view.feedback}</p>}
    </Step>
  );
}
