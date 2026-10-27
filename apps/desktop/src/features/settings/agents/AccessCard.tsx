import type { AccessConfig } from "../../../bridge/generated/AccessConfig";
import { Switch } from "../../../design/Switch";
import { accessGroups } from "../../../i18n/es/agents";
import { Group, List, Row } from "../layout";
import { copy, FeedbackLine, NotEditable, type OrderProps } from "./parts";

/**
 * «Qué puede usar»: one switch per group of Orbit's tools, each named by
 * what it lets the agent do. A switch is configuration: it is sent as it is
 * flipped, with all access groups, and nothing asks again.
 */
export function AccessCard({
  access,
  explained,
  agentId,
  busy,
  feedback,
  order,
}: OrderProps & { access: AccessConfig; explained: boolean }) {
  return (
    <Group help={copy.accessHelp} label={copy.access}>
      {!explained && <NotEditable cause={access.cause} editable={access.editable} />}
      <List>
        {accessGroups.map(({ group, label, description }) => (
          <Row
            aside={
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
            }
            help={description}
            key={group}
            title={label}
          />
        ))}
      </List>
      <FeedbackLine feedback={feedback} place="access" />
    </Group>
  );
}
