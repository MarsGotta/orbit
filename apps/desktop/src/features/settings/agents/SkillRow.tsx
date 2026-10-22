import { useState } from "react";
import type { SkillSummary } from "../../../bridge/generated/SkillSummary";
import { Button } from "../../../design/Button";
import { Switch } from "../../../design/Switch";
import styles from "./Agents.module.css";
import { copy, type OrderProps } from "./parts";

type RowProps = Omit<OrderProps, "feedback"> & {
  skill: SkillSummary;
  editable: boolean;
  onOpen: (name: string) => void;
};

/** Which skill before it goes: the hub keeps no copy, so there is nothing to undo. */
function DeleteQuestion(props: {
  busy: boolean;
  onCancel: () => void;
  onDelete: () => void;
  skill: string;
}) {
  return (
    <div className={styles.confirm} role="alert">
      <span>{copy.deleteQuestion(props.skill)}</span>
      <Button disabled={props.busy} onClick={props.onDelete} size={28} variant="dangerSoft">
        {copy.confirmDelete}
      </Button>
      <Button onClick={props.onCancel} size={28} variant="ghost">
        {copy.cancel}
      </Button>
    </div>
  );
}

/**
 * One skill of the agent: switched on or off as flipped, opened to read or
 * edit, and deleted only after the row says which one: the hub keeps no copy,
 * so there is nothing to undo.
 */
export function SkillRow({ skill, editable, agentId, busy, order, onOpen }: RowProps) {
  const [deleting, setDeleting] = useState(false);
  return (
    <div className={styles.skill}>
      <div className={styles.skillMain}>
        <div className={styles.skillText}>
          <span className={styles.skillName}>{skill.name}</span>
          <span className={styles.skillHelp}>{skill.description}</span>
        </div>
        <Button
          aria-label={editable ? copy.edit(skill.name) : copy.view(skill.name)}
          disabled={busy}
          onClick={() => onOpen(skill.name)}
          size={28}
          variant="ghost"
        >
          {editable ? copy.editLabel : copy.viewLabel}
        </Button>
        {editable && (
          <Button
            aria-label={copy.delete(skill.name)}
            disabled={busy}
            onClick={() => setDeleting(true)}
            size={28}
            variant="ghost"
          >
            {copy.deleteLabel}
          </Button>
        )}
        <Switch
          checked={skill.enabled}
          disabled={!editable || busy}
          label={copy.enable(skill.name)}
          onCheckedChange={(enabled) =>
            void order(
              { call: "skill_enabled", agent_id: agentId, name: skill.name, enabled },
              "skills",
            )
          }
        />
      </div>
      {deleting && (
        <DeleteQuestion
          busy={busy}
          onCancel={() => setDeleting(false)}
          onDelete={() => {
            setDeleting(false);
            void order({ call: "skill_delete", agent_id: agentId, name: skill.name }, "skills");
          }}
          skill={skill.name}
        />
      )}
    </div>
  );
}
