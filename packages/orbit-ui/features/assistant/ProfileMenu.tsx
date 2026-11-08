import { getCopy } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import { useState } from "react";
import type { Agent } from "../../bridge/generated/Agent";
import { AssistantAvatar, type AssistantColor } from "../../design/AssistantAvatar";
import { Button } from "../../design/Button";
import { EditMenu } from "../../design/EditMenu";
import type { Feedback } from "../../design/feedback";
import { Icon } from "../../design/Icon";
import { Switch } from "../../design/Switch";

import type { AgentPort } from "../settings/agents/use-agent-config";
import { callAgent } from "./agent-calls";
import { assistantHash } from "./assistant-route";
import { colorOf, profileProblem } from "./assistants";
import { ColorField, DescriptionField, NameField } from "./ProfileFields";
import styles from "./AssistantPages.module.css";

interface Profile {
  name: string;
  description: string;
  color: AssistantColor;
  primary: boolean;
}

/** The words under the primary switch: what will happen, with whom it is now. */
function primaryHelp(chosen: boolean, isPrimary: boolean, current?: Agent): string {
  if (isPrimary)
    return `${getCopy().extras.assistantsCopy.primaryIs} ${getCopy().extras.assistantsCopy.primaryKeep}`;
  if (chosen)
    return current === undefined
      ? getCopy().extras.assistantsCopy.primaryIs
      : getCopy().extras.assistantsCopy.primaryWill(current.name);
  return current === undefined ? "" : getCopy().extras.assistantsCopy.primaryNow(current.name);
}

/** What is being written, whether it is a valid change, and «Guardar» that sends all four. */
function useProfileDraft(agent: Agent, agents: readonly Agent[], primaryId?: string) {
  const initial: Profile = {
    name: agent.name,
    description: agent.description ?? "",
    color: colorOf(agent, agents),
    primary: agent.id === primaryId,
  };
  const [draft, setDraft] = useState(initial);
  const [feedback, setFeedback] = useState<Feedback>("idle");
  const [refusal, setRefusal] = useState<string | undefined>(undefined);
  const problem = profileProblem(draft.name, agent.id, agents);
  const changed =
    draft.name.trim() !== initial.name ||
    draft.description.trim() !== initial.description ||
    draft.color !== initial.color ||
    draft.primary !== initial.primary;
  /** `true` once the hub took it; a refusal is said in the menu. */
  const save = async (port?: AgentPort): Promise<boolean> => {
    if (!changed || problem !== null) return false;
    setFeedback("pending");
    setRefusal(undefined);
    const result = await callAgent(port, {
      call: "profile",
      agent_id: agent.id,
      name: draft.name.trim(),
      description: draft.description.trim(),
      color: `c${draft.color}`,
      primary: draft.primary,
    });
    setFeedback("idle");
    if (!result.ok) setRefusal(result.text);
    return result.ok;
  };
  const change = (part: Partial<Profile>) => setDraft((now) => ({ ...now, ...part }));
  return { draft, change, problem, changed, feedback, refusal, save };
}

/** «Más ajustes»: the gear's page, from the pencil. */
function MoreSettings({ agentId, onClose }: { agentId: string; onClose: () => void }) {
  const copy = useCopy().extras.assistantsCopy;
  return (
    <Button
      onClick={() => {
        onClose();
        window.location.hash = assistantHash({ screen: "settings", agentId });
      }}
      size={32}
      variant="ghost"
    >
      <Icon name="gear" size={14} />
      {copy.moreSettings}
    </Button>
  );
}

/** «Asistente principal» with what will happen; a primary stays one until another is made it. */
function PrimaryField({
  chosen,
  isPrimary,
  current,
  onChange,
}: {
  chosen: boolean;
  isPrimary: boolean;
  current?: Agent;
  onChange: (primary: boolean) => void;
}) {
  const copy = useCopy().extras.assistantsCopy;
  return (
    <div className={styles.primaryRow}>
      <Switch checked={chosen} disabled={isPrimary} label={copy.primary} onCheckedChange={onChange}>
        <span className={styles.primaryText}>
          <span className={styles.primaryTitle}>{copy.primary}</span>
          <span className={styles.fieldHint}>{primaryHelp(chosen, isPrimary, current)}</span>
        </span>
      </Switch>
    </div>
  );
}

/**
 * The pencil (proposal 27): name, description, colour and primary, saved
 * in the hub so both Macs see the same. A name empty or another's (ignoring
 * case and accents) cannot be saved; Esc and a press outside save nothing;
 * ↵ in the name saves. A primary stays one until another is made primary.
 */
interface ProfileMenuProps {
  agent: Agent;
  agents: readonly Agent[];
  primaryId?: string;
  anchor: HTMLElement | null;
  port?: AgentPort;
  /** «Más ajustes»: not in the settings page itself. */
  showMore: boolean;
  onClose: () => void;
  onSaved: () => void;
}

export function ProfileMenu(props: ProfileMenuProps) {
  const copy = useCopy().extras.assistantsCopy;
  const { agent, agents, primaryId, anchor, port, showMore, onClose, onSaved } = props;
  const { draft, change, problem, changed, feedback, refusal, save } = useProfileDraft(
    agent,
    agents,
    primaryId,
  );
  const isPrimary = agent.id === primaryId;
  const line = copy.serviceFull(agent.service);
  return (
    <EditMenu
      anchor={anchor}
      canSave={changed && problem === null}
      error={refusal}
      extra={showMore ? <MoreSettings agentId={agent.id} onClose={onClose} /> : undefined}
      feedback={feedback}
      leading={
        <AssistantAvatar
          color={draft.color}
          name={draft.name.trim() === "" ? agent.name : draft.name}
          size={40}
        />
      }
      onClose={onClose}
      onSave={() =>
        void save(port).then((taken) => {
          if (!taken) return;
          onSaved();
          onClose();
        })
      }
      subtitle={line === undefined ? undefined : copy.worksWith(line)}
      title={copy.editTitle(agent.name)}
      width={400}
    >
      <NameField onChange={(name) => change({ name })} problem={problem} value={draft.name} />
      <DescriptionField
        onChange={(description) => change({ description })}
        value={draft.description}
      />
      <ColorField onChange={(color) => change({ color })} value={draft.color} />
      <PrimaryField
        chosen={draft.primary}
        current={agents.find((each) => each.id === primaryId)}
        isPrimary={isPrimary}
        onChange={(primary) => change({ primary })}
      />
      {agent.service === "hermes" && <p className={styles.fieldHint}>{copy.hubManaged}</p>}
    </EditMenu>
  );
}
