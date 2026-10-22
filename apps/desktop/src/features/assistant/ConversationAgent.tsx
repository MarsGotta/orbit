import type { AppCommand } from "../../bridge/generated";
import { es } from "../../i18n/es";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { useHeldChoice } from "../../state/use-held-choice";
import { agentFor, agentsOf } from "./agents";
import styles from "./AssistantView.module.css";

/**
 * With several agents, whose the conversation is: a new one, before its
 * first message, picks its agent; an open one says it and keeps it. With
 * one agent there is nothing to say.
 */
export function ConversationAgent({
  snapshot,
  send,
}: {
  snapshot: AppSnapshot;
  send: (command: AppCommand) => unknown;
}) {
  const agents = agentsOf(snapshot);
  const { chat } = snapshot;
  const current = agentFor(snapshot, chat.agent_id);
  // The agent chosen stays shown until the new conversation says it.
  const agent = useHeldChoice(current?.id, send);
  if (agents.length < 2 || !current) return null;
  if (chat.session_id !== null || chat.turn.length > 0)
    return <span>{es.assistant.conversationWith(current.name)}</span>;
  return (
    <label className={styles.agentPicker}>
      <span>{es.assistant.agentPicker}</span>
      <select
        onChange={(event) =>
          agent.choose(event.target.value, { command: "new_session", agent_id: event.target.value })
        }
        value={agent.shown ?? current.id}
      >
        {agents.map((agent) => (
          <option key={agent.id} value={agent.id}>
            {agent.name}
          </option>
        ))}
      </select>
    </label>
  );
}
