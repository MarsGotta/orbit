import { useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { Agent } from "../../bridge/generated/Agent";
import { Breadcrumbs, TopBarLeading } from "../../app/shell-slots";
import { AssistantAvatar } from "../../design/AssistantAvatar";
import { Button } from "../../design/Button";
import { useFeedback } from "../../design/feedback";
import { assistantsCopy as copy } from "../../i18n/es/assistants";
import { es } from "../../i18n/es";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { AgentCards } from "../settings/agents/AgentCards";
import type { AgentPort } from "../settings/agents/use-agent-config";
import { ControlHistory } from "../settings/ControlHistory";
import { Group, List, Row, SkeletonRows } from "../settings/layout";
import { callAgent } from "./agent-calls";
import { assistantHash, goTo } from "./assistant-route";
import { colorOf } from "./assistants";
import { Connection } from "./AssistantConnection";
import { agentsOf } from "./agents";
import { AssistantIdentity } from "./AssistantIdentity";
import styles from "./Assistants.module.css";

type Send = (command: AppCommand) => unknown;

/** «Quitar a … de Orbit», asked in its own row; one the hub gives says who can. */
function Removal({ agent, port }: { agent: Agent; port?: AgentPort }) {
  const [asking, setAsking] = useState(false);
  const removing = useFeedback();
  const [refusal, setRefusal] = useState<string | null>(null);
  if (agent.removable !== true)
    return <p className={styles.removeNote}>{copy.notRemovable(agent.name)}</p>;
  const remove = () => {
    setRefusal(null);
    const work = callAgent(port, { call: "remove", agent_id: agent.id }).then((result) => {
      if (!result.ok) {
        setRefusal(result.text);
        throw new Error(result.text);
      }
      goTo({ screen: "conversation" });
    });
    removing.track(work);
  };
  return (
    <div className={styles.removal}>
      {asking ? (
        <div className={styles.removeAsk} role="group" aria-label={copy.removeQuestion(agent.name)}>
          <p className={styles.removeTitle}>{copy.removeQuestion(agent.name)}</p>
          <p className={styles.removeText}>{copy.removeText}</p>
          <div className={styles.actionsRow}>
            <Button onClick={() => setAsking(false)} size={32} variant="ghost">
              {es.controls.cancel}
            </Button>
            <Button
              feedback={removing.feedback}
              onClick={remove}
              pendingLabel={copy.removing}
              size={32}
              variant="dangerSoft"
            >
              {copy.removeConfirm}
            </Button>
          </div>
          {refusal !== null && (
            <p className={styles.fieldError} role="alert">
              {refusal}
            </p>
          )}
        </div>
      ) : (
        <Button onClick={() => setAsking(true)} size={32} variant="dangerSoft">
          {copy.remove(agent.name)}
        </Button>
      )}
    </div>
  );
}

/** Its avatar, «Ajustes de …» with the pencil, its description and its service. */
function SettingsHead({
  agent,
  snapshot,
  port,
}: {
  agent: Agent;
  snapshot: AppSnapshot;
  port?: AgentPort;
}) {
  const agents = agentsOf(snapshot);
  const line = copy.serviceFull(agent.service);
  return (
    <header className={styles.settingsHead}>
      <AssistantAvatar color={colorOf(agent, agents)} name={agent.name} size={56} />
      <div className={styles.settingsName}>
        <div className={styles.settingsTitleRow}>
          <h1>{copy.settingsTitle(agent.name)}</h1>
          {agent.id === snapshot.settings.default_agent_id && agents.length > 1 && (
            <span className={styles.pill}>{copy.principal}</span>
          )}
          <AssistantIdentity
            agent={agent}
            heading={false}
            inSettings
            port={port}
            snapshot={snapshot}
          />
        </div>
        {agent.description !== undefined && agent.description !== "" && <p>{agent.description}</p>}
        {line !== undefined && <p className={styles.settingsService}>{copy.worksWith(line)}</p>}
      </div>
    </header>
  );
}

/** «En Orbit»: how it is called in a meeting, and whose voice reads it. */
function InOrbit({ agent, snapshot }: { agent: Agent; snapshot: AppSnapshot }) {
  const primary = agentsOf(snapshot).find((each) => each.id === snapshot.settings.default_agent_id);
  const primaryName = primary?.name ?? snapshot.settings.assistant_name ?? es.chat.roles.assistant;
  return (
    <Group label={copy.inOrbit}>
      <List>
        <Row help={copy.inMeetingText(agent.name)} title={copy.inMeeting} />
        {agent.voice !== undefined && (
          <Row
            help={agent.voice ? copy.ownVoice : copy.borrowedVoice(primaryName)}
            title={copy.voice}
          />
        )}
      </List>
    </Group>
  );
}

/** An id the hub no longer has (removed), or the agents still coming. */
function NoSuchAssistant({ known }: { known: boolean }) {
  return (
    <div className={styles.settingsScroll}>
      <div className={styles.settingsPage}>
        {known ? (
          <p className={styles.removeNote}>
            {copy.unknownAssistant}{" "}
            <a href={assistantHash({ screen: "conversation" })}>{copy.backToConversation}</a>
          </p>
        ) : (
          <SkeletonRows rows={4} />
        )}
      </div>
    </div>
  );
}

/**
 * An assistant's settings, behind its gear (annex 37–39): what Ajustes ›
 * Asistente showed for it, with its connection and its plan's usage. Its
 * name, description, colour and whether it is the primary are the pencil's.
 * What its assistants did on this Mac is everyone's: the hub does not yet
 * say which assistant asked for each action.
 */
export function AssistantSettingsPage({
  agentId,
  snapshot,
  port,
  send,
  nowMs,
}: {
  agentId: string;
  snapshot: AppSnapshot;
  port?: AgentPort;
  send: Send;
  nowMs: number;
}) {
  const agents = agentsOf(snapshot);
  const agent = agents.find((each) => each.id === agentId);
  if (agent === undefined) return <NoSuchAssistant known={agents.length > 0} />;
  const back = {
    label: copy.backTo(agent.name),
    href: assistantHash({ screen: "agent", agentId }),
  };
  return (
    <>
      <TopBarLeading>
        <Breadcrumbs trail={[back, { label: copy.settingsCrumb }]} />
      </TopBarLeading>
      <div className={styles.settingsScroll}>
        <div className={styles.settingsPage}>
          <SettingsHead agent={agent} port={port} snapshot={snapshot} />
          <p className={styles.settingsLead}>{copy.settingsLead}</p>
          <Connection agent={agent} nowMs={nowMs} port={port} send={send} snapshot={snapshot} />
          {port === undefined ? (
            <p className={styles.removeNote}>{es.agents.unavailable}</p>
          ) : (
            <AgentCards agent={port} key={agent.id} target={agent} />
          )}
          <InOrbit agent={agent} snapshot={snapshot} />
          <Group help={copy.historyShared} label={copy.history}>
            <ControlHistory control={snapshot.computer_control} />
          </Group>
          <Removal agent={agent} port={port} />
        </div>
      </div>
    </>
  );
}
