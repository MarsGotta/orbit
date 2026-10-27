import { useState } from "react";
import type { AgentPort } from "../settings/agents/use-agent-config";
import { AIProviderEditor } from "../settings/AIProviderEditor";
import { useAISettings } from "../settings/use-ai-settings";
import { Step } from "./OnboardingSteps";

export function AIStep({ agent }: { agent?: AgentPort }) {
  const [add, setAdd] = useState(false);
  const view = useAISettings(agent, false);
  return (
    <Step
      title="Tu IA"
      lead="Puedes empezar con tu hub o añadir una clave en la nube o un endpoint compatible con OpenAI."
    >
      <label>
        <input type="radio" name="ai-setup" checked={!add} onChange={() => setAdd(false)} />
        Todo en tu hub
      </label>
      <p>
        Esta opción no añade proveedores ni cambia la configuración del operador de tu hub. Puedes
        revisarla en Ajustes › IA.
      </p>
      <label>
        <input type="radio" name="ai-setup" checked={add} onChange={() => setAdd(true)} />
        Añadir un proveedor
      </label>
      {add && <AIProviderEditor view={view} onClose={() => setAdd(false)} />}
      {view.feedback.length > 0 && <p role="status">{view.feedback}</p>}
    </Step>
  );
}
