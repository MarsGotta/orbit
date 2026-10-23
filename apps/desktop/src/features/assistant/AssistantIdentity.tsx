import { useEffect, useRef, useState } from "react";
import type { Agent } from "../../bridge/generated/Agent";
import { AssistantAvatar } from "../../design/AssistantAvatar";
import { IconButton } from "../../design/IconButton";
import { assistantsCopy as copy } from "../../i18n/es/assistants";
import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AgentPort } from "../settings/agents/use-agent-config";
import { assistantHash } from "./assistant-route";
import { colorOf } from "./assistants";
import { agentsOf } from "./agents";
import { ProfileMenu } from "./ProfileMenu";
import styles from "./AssistantView.module.css";

/** How long «✓ Guardado» stays beside the name after the pencil saved (annex: 2,5 s). */
const SAVED_MS = 2_500;

/** «✓ Guardado» for 2.5 s after `saved()`; an empty status the rest of the time. */
function useSavedEcho() {
  const [shown, setShown] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  const saved = () => {
    window.clearTimeout(timer.current);
    setShown(true);
    timer.current = window.setTimeout(() => setShown(false), SAVED_MS);
  };
  return { shown, saved };
}

/** The pencil and its menu, the gear, and «✓ Guardado» for 2.5 s after the pencil saved. */
function PencilAndGear({
  agent,
  snapshot,
  port,
  inSettings,
}: {
  agent: Agent;
  snapshot: AppSnapshot;
  port?: AgentPort;
  inSettings: boolean;
}) {
  const pencil = useRef<HTMLButtonElement>(null);
  const [editing, setEditing] = useState(false);
  const echo = useSavedEcho();
  return (
    <>
      {agent.service !== undefined && (
        <IconButton
          aria-expanded={editing}
          aria-haspopup="dialog"
          aria-label={copy.editLabel(agent.name)}
          data-open={editing || undefined}
          icon="edit"
          label={copy.editTip}
          onClick={() => setEditing((open) => !open)}
          ref={pencil}
          size={30}
          tip="below"
        />
      )}
      {!inSettings && (
        <IconButton
          icon="gear"
          label={copy.settingsTip(agent.name)}
          onClick={() => {
            window.location.hash = assistantHash({ screen: "settings", agentId: agent.id });
          }}
          size={30}
          tip="below"
        />
      )}
      <span className={styles.savedEcho} role="status">
        {echo.shown ? copy.saved : ""}
      </span>
      {editing && (
        <ProfileMenu
          agent={agent}
          agents={agentsOf(snapshot)}
          anchor={pencil.current}
          onClose={() => setEditing(false)}
          onSaved={echo.saved}
          port={port}
          primaryId={snapshot.settings.default_agent_id}
          showMore={!inSettings}
        />
      )}
    </>
  );
}

/**
 * Who it is, with the pencil (its name and description) and, outside its
 * own settings, the gear: the avatar, the name, the service in a pill and
 * what it is doing.
 */
export function AssistantIdentity({
  agent,
  snapshot,
  port,
  state,
  tone,
  inSettings = false,
  heading = true,
}: {
  agent: Agent;
  snapshot: AppSnapshot;
  port?: AgentPort;
  state?: string;
  tone?: "warn";
  inSettings?: boolean;
  /** The name as the view's heading; the settings page has its own. */
  heading?: boolean;
}) {
  const service = copy.serviceShort(agent.service);
  return (
    <span className={styles.identity}>
      {heading && (
        <>
          <AssistantAvatar color={colorOf(agent, agentsOf(snapshot))} name={agent.name} size={26} />
          <h2>{agent.name}</h2>
          {service !== undefined && <span className={styles.servicePill}>{service}</span>}
        </>
      )}
      <PencilAndGear agent={agent} inSettings={inSettings} port={port} snapshot={snapshot} />
      {state !== undefined && state !== "" && (
        <span className={styles.headerState} data-tone={tone}>
          {state}
        </span>
      )}
    </span>
  );
}
