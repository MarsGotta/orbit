import { useCopy } from "../../i18n/CopyProvider";
import type { AppView } from "../../state/app-view";

import { agentsOf } from "../assistant/agents";
import type { AgentPort } from "./agents/use-agent-config";
import { AIFunctions } from "./AIFunctions";
import { AIProviders } from "./AIProviders";
import { Group, List, Row, SectionHeading } from "./layout";
import { useAISettings } from "./use-ai-settings";

export function AISettings({ agent, snapshot }: { agent?: AgentPort; snapshot?: AppView }) {
  const inlineCopy = useCopy().settingsInline;
  const copy = useCopy().extras.aiCopy;
  const view = useAISettings(agent);
  const subscriptions =
    snapshot === undefined
      ? []
      : agentsOf(snapshot).filter((a) => a.service === "codex" || a.service === "claude-code");
  return (
    <>
      <SectionHeading title={copy.title} lead={copy.lead} />
      <AIProviders view={view} />
      <Group label={inlineCopy.subscriptions} help={inlineCopy.subscriptionsHelp}>
        <List>
          {subscriptions.map((a) => (
            <Row
              key={a.id}
              title={a.name}
              help={
                a.account === "signed_in"
                  ? inlineCopy.signedIn
                  : a.account === "expired"
                    ? inlineCopy.expired
                    : a.account === "signed_out"
                      ? inlineCopy.signedOut
                      : inlineCopy.unknownAccount
              }
            />
          ))}
          <Row title={<a href="#/settings/asistentes">{inlineCopy.assistants}</a>} />
        </List>
      </Group>
      <AIFunctions view={view} />
      {view.errorPlace === "load" && view.error.length > 0 && <p role="alert">{view.error}</p>}
      {view.feedback.length > 0 && <p role="status">{view.feedback}</p>}
    </>
  );
}
