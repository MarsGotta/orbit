import { useCopy } from "../../i18n/CopyProvider";
import { useId } from "react";
import { AssistantAvatar } from "../../design/AssistantAvatar";
import { Icon } from "../../design/Icon";
import { Tooltip } from "../../design/Tooltip";

import { useNow } from "../../use-now";
import type { AppView as AppSnapshot } from "../../state/app-view";
import { assistantHash, useAssistantRoute } from "./assistant-route";
import { colorOf, rowWarning } from "./assistants";
import { agentsOf } from "./agents";
import styles from "./Sessions.module.css";

// A limit's time passes while the column is open: read the clock each minute.
const REPAINT_MS = 60_000;

/** «Tus asistentes»: each one with its service, and what is wrong when something is. */
export function YourAssistants({ snapshot }: { snapshot: AppSnapshot }) {
  const assistantsCopy = useCopy().extras.assistantsCopy;
  const titleId = useId();
  const route = useAssistantRoute();
  const now = useNow(REPAINT_MS);
  const agents = agentsOf(snapshot);
  if (agents.length === 0) return null;
  const here = route.screen === "agent" || route.screen === "settings" ? route.agentId : undefined;
  return (
    <section className={styles.yours}>
      <div className={styles.groupHead}>
        <h3 className={styles.group} id={titleId}>
          {assistantsCopy.yours}
        </h3>
        {snapshot.settings.addable_agents !== undefined && (
          <Tooltip text={assistantsCopy.add}>
            <a
              aria-label={assistantsCopy.add}
              className={styles.add}
              href={assistantHash({ screen: "add" })}
            >
              <Icon name="plus" size={14} />
            </a>
          </Tooltip>
        )}
      </div>
      <ul aria-labelledby={titleId} className={styles.list}>
        {agents.map((agent) => {
          const warning = rowWarning(agent, now);
          return (
            <li key={agent.id}>
              <a
                aria-current={agent.id === here ? "page" : undefined}
                className={styles.assistant}
                href={assistantHash({ screen: "agent", agentId: agent.id })}
              >
                <AssistantAvatar color={colorOf(agent, agents)} name={agent.name} size={28} />
                <span className={styles.assistantText}>
                  <span className={styles.assistantName}>{agent.name}</span>
                  {assistantsCopy.serviceShort(agent.service) !== undefined && (
                    <span className={styles.assistantService}>
                      {assistantsCopy.serviceShort(agent.service)}
                    </span>
                  )}
                </span>
                {warning !== null && (
                  <span className={styles.warning}>
                    <span aria-hidden="true" className={styles.warningDot} />
                    {warning}
                  </span>
                )}
              </a>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
