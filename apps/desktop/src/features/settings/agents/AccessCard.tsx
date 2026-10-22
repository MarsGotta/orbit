import type { AccessConfig } from "../../../bridge/generated/AccessConfig";
import { Switch } from "../../../design/Switch";
import { accessGroups } from "../../../i18n/es/agents";
import settingsStyles from "../SettingsView.module.css";
import { ACCESS_GROUP_TOOLS } from "./access-tools";
import styles from "./Agents.module.css";
import { copy, FeedbackLine, NotEditable, type OrderProps, Panel } from "./parts";

/**
 * One switch per group of Orbit's tools, with the tools it opens. A switch
 * is configuration: it is sent as it is flipped, with all five groups, and
 * nothing asks again.
 */
export function AccessCard({
  access,
  agentId,
  busy,
  feedback,
  order,
}: OrderProps & { access: AccessConfig }) {
  return (
    <Panel>
      <h3>{copy.access}</h3>
      <p className={styles.hint}>{copy.accessHelp}</p>
      <NotEditable editable={access.editable} cause={access.cause} />
      <div className={settingsStyles.controls}>
        {accessGroups.map(({ group, label }) => (
          <div className={settingsStyles.controlRow} key={group}>
            <div className={settingsStyles.rowCopy}>
              <strong>{label}</strong>
              <span>{ACCESS_GROUP_TOOLS[group].join(", ")}</span>
            </div>
            <Switch
              checked={access.groups[group]}
              disabled={!access.editable || busy}
              label={label}
              onCheckedChange={(checked) =>
                void order(
                  {
                    call: "access",
                    agent_id: agentId,
                    groups: { ...access.groups, [group]: checked },
                  },
                  "access",
                )
              }
            />
          </div>
        ))}
      </div>
      <FeedbackLine feedback={feedback} place="access" />
    </Panel>
  );
}
