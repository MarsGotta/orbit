import type { AppCommand } from "../../bridge/generated";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { es as base } from "../../i18n/es";
import { agentsOf } from "../assistant/agents";
import { AgentSettings } from "./agents/AgentSettings";
import type { AgentPort } from "./agents/use-agent-config";
import { AssistantFolders } from "./AssistantFolders";
import { ControlHistory } from "./ControlHistory";
import { Group, SectionHeading } from "./layout";

/** Ajustes › Asistente: each agent, the folders they may read and what they did here. */
export function AssistantSection({
  agent,
  send,
  snapshot,
}: {
  agent?: AgentPort;
  send: (command: AppCommand) => unknown;
  snapshot: AppSnapshot;
}) {
  return (
    <>
      <SectionHeading
        lead={base.settings.leads.asistente}
        title={base.shell.settingsSections.asistente}
      />
      <AgentSettings
        agent={agent}
        agents={agentsOf(snapshot)}
        defaultAgentId={snapshot.settings.default_agent_id}
      />
      <AssistantFolders
        daemonReady={snapshot.connection.daemon === "ready"}
        folders={snapshot.computer_control?.folders}
        privacyStopped={snapshot.settings.privacy_stopped}
        send={send}
      />
      <Group label={base.settings.controlHistory}>
        <ControlHistory control={snapshot.computer_control} />
      </Group>
    </>
  );
}
