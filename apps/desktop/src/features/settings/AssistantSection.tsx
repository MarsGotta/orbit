import { rowState } from "./assistant-row-state";
import { useCopy } from "../../i18n/CopyProvider";
import { useId } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { Agent } from "../../bridge/generated/Agent";
import { AssistantAvatar } from "../../design/AssistantAvatar";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";

import type { AppView as AppSnapshot } from "../../state/app-view";
import { useNow } from "../../use-now";
import { assistantHash } from "../assistant/assistant-route";
import { colorOf } from "../assistant/assistants";
import { agentsOf } from "../assistant/agents";
import type { AgentPort } from "./agents/use-agent-config";
import { AssistantFolders } from "./AssistantFolders";
import { ControlHistory } from "./ControlHistory";
import { Group, List, Row, SectionHeading } from "./layout";
import { usePrimary } from "./use-primary";
import styles from "./SettingsView.module.css";

// A limit's time passes while the section is open: read the clock each minute.
const REPAINT_MS = 60_000;

/** Leaves Settings for the Assistant's own screens. */
function goTo(hash: string) {
  window.location.hash = hash;
}

/** One row per assistant with its service and state, «Sus ajustes» to its gear, and «Añadir asistente». */
function AssistantRows({ snapshot }: { snapshot: AppSnapshot }) {
  const copy = useCopy().settings;
  const assistantsCopy = useCopy().extras.assistantsCopy;
  const titleId = useId();
  const nowMs = useNow(REPAINT_MS);
  const agents = agentsOf(snapshot);
  const primaryId = snapshot.settings.default_agent_id;
  return (
    <div className={`${styles.list} ${styles.group}`}>
      <h3 className={styles.visuallyHidden} id={titleId}>
        {copy.yourAssistants}
      </h3>
      <ul aria-labelledby={titleId} className={styles.assistantRows}>
        {agents.map((agent) => {
          const state = rowState(agent, nowMs);
          const service = assistantsCopy.serviceFull(agent.service);
          return (
            <li className={styles.assistantRow} key={agent.id}>
              <AssistantAvatar color={colorOf(agent, agents)} name={agent.name} size={36} />
              <span className={styles.rowText}>
                <strong className={styles.rowTitle}>{agent.name}</strong>
                {service !== undefined && (
                  <span className={styles.rowHelp}>{assistantsCopy.worksWith(service)}</span>
                )}
              </span>
              {agent.id === primaryId && agents.length > 1 && (
                <span className={styles.pill}>{assistantsCopy.primaryBadge}</span>
              )}
              {state !== null && (
                <span
                  className={state.warn ? styles.rowWarn : styles.meta}
                  data-tone={state.warn ? "warn" : undefined}
                >
                  {state.warn && <span aria-hidden="true" className={styles.dot} />}
                  {state.text}
                </span>
              )}
              <Button
                onClick={() => goTo(assistantHash({ screen: "settings", agentId: agent.id }))}
                size={32}
              >
                {copy.theirSettings}
              </Button>
            </li>
          );
        })}
      </ul>
      {snapshot.settings.addable_agents !== undefined && (
        <div className={styles.padded}>
          <Button onClick={() => goTo(assistantHash({ screen: "add" }))} size={32}>
            <Icon name="plus" size={14} />
            {assistantsCopy.add}
          </Button>
        </div>
      )}
    </div>
  );
}

/** «Asistente principal», a selector; the hub keeps the choice. */
function PrimaryChoice({
  agents,
  port,
  primaryId,
}: {
  agents: readonly Agent[];
  port?: AgentPort;
  primaryId: string | null | undefined;
}) {
  const copy = useCopy().settings;
  const assistantsCopy = useCopy().extras.assistantsCopy;
  const id = useId();
  const primary = usePrimary(port, agents, primaryId);
  return (
    <Group help={copy.primaryHelp} label={assistantsCopy.primary}>
      <List>
        <Row
          aside={
            <span className={styles.select}>
              <select
                disabled={primary.saving}
                id={id}
                onChange={(event) => void primary.choose(event.currentTarget.value)}
                value={primary.current ?? ""}
              >
                {primary.current === null && <option value="" />}
                {agents.map((agent) => (
                  <option key={agent.id} value={agent.id}>
                    {agent.name}
                  </option>
                ))}
              </select>
              <Icon name="chevron" size={14} />
            </span>
          }
          help={
            primary.saving ? (
              <span role="status">{copy.primarySaving}</span>
            ) : primary.refusal !== null ? (
              <span data-tone="warn" role="alert">
                {primary.refusal}
              </span>
            ) : undefined
          }
          title={<label htmlFor={id}>{copy.primaryLabel}</label>}
        />
      </List>
    </Group>
  );
}

/**
 * Settings › Assistants, a summary: each assistant is edited from its
 * conversation (the pencil) and its gear (`#/assistant/<id>/settings`).
 * Below, the primary, the folders all of them may read and what they did on
 * this Mac. The hub does not yet say which assistant asked for each action,
 * so the history carries no avatars and says why.
 */
export function AssistantSection({
  agent,
  send,
  snapshot,
}: {
  agent?: AgentPort;
  send: (command: AppCommand) => unknown;
  snapshot: AppSnapshot;
}) {
  const copy = useCopy().settings;
  const base = useCopy();
  const assistantsCopy = useCopy().extras.assistantsCopy;
  const agents = agentsOf(snapshot);
  return (
    <>
      <SectionHeading lead={copy.leads.assistants} title={base.shell.settingsSections.assistants} />
      {agents.length === 0 ? (
        <List>
          <p className={styles.empty}>{base.agents.none}</p>
        </List>
      ) : (
        <>
          <AssistantRows snapshot={snapshot} />
          <PrimaryChoice
            agents={agents}
            port={agent}
            primaryId={snapshot.settings.default_agent_id}
          />
        </>
      )}
      <AssistantFolders
        daemonReady={snapshot.connection.daemon === "ready"}
        folders={snapshot.computer_control?.folders}
        privacyStopped={snapshot.settings.privacy_stopped}
        send={send}
      />
      <Group help={assistantsCopy.historyShared} label={copy.controlHistory}>
        <ControlHistory control={snapshot.computer_control} />
      </Group>
    </>
  );
}
