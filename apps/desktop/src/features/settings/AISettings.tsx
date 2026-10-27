import type { AppView } from "../../state/app-view";
import { aiCopy as copy } from "../../i18n/es/ai";
import { agentsOf } from "../assistant/agents";
import type { AgentPort } from "./agents/use-agent-config";
import { AIFunctions } from "./AIFunctions";
import { AIProviders } from "./AIProviders";
import { Group, List, Row, SectionHeading } from "./layout";
import { useAISettings } from "./use-ai-settings";

export function AISettings({ agent, snapshot }: { agent?: AgentPort; snapshot?: AppView }) {
  const view = useAISettings(agent);
  const subscriptions =
    snapshot === undefined
      ? []
      : agentsOf(snapshot).filter((a) => a.service === "codex" || a.service === "claude-code");
  return (
    <>
      <SectionHeading title={copy.title} lead={copy.lead} />
      <AIProviders view={view} />
      <Group
        label="Suscripciones"
        help="Claude y ChatGPT sólo sirven a sus asistentes. Sus cuentas se gestionan en Asistentes."
      >
        <List>
          {subscriptions.map((a) => (
            <Row
              key={a.id}
              title={a.name}
              help={
                a.account === "signed_in"
                  ? "Conectada"
                  : a.account === "expired"
                    ? "Sesión caducada"
                    : a.account === "signed_out"
                      ? "Sin conectar"
                      : "Estado de cuenta desconocido"
              }
            />
          ))}
          <Row title={<a href="#/settings/asistentes">Ir a Asistentes</a>} />
        </List>
      </Group>
      <AIFunctions view={view} />
      {view.errorPlace === "load" && view.error.length > 0 && <p role="alert">{view.error}</p>}
      {view.feedback.length > 0 && <p role="status">{view.feedback}</p>}
    </>
  );
}
