import { useState } from "react";
import type { SkillSummary } from "../../../bridge/generated/SkillSummary";
import { Button } from "../../../design/Button";
import { Switch } from "../../../design/Switch";
import settingsStyles from "../SettingsView.module.css";
import styles from "./Agents.module.css";
import { copy, type OrderProps } from "./parts";

type RowProps = Omit<OrderProps, "feedback"> & {
  skill: SkillSummary;
  editable: boolean;
  onOpen: (name: string) => void;
};

/** The row says which skill before it is deleted: the hub keeps no copy. */
function DeleteQuestion({
  skill,
  agentId,
  busy,
  order,
  onDone,
}: Omit<RowProps, "editable" | "onOpen"> & { onDone: () => void }) {
  return (
    <span className={styles.row}>
      {copy.deleteQuestion(skill.name)}
      <Button
        disabled={busy}
        onClick={() => {
          onDone();
          void order({ call: "skill_delete", agent_id: agentId, name: skill.name }, "skills");
        }}
        size={28}
      >
        {copy.confirmDelete}
      </Button>
      <Button onClick={onDone} size={28} variant="ghost">
        {copy.cancel}
      </Button>
    </span>
  );
}

/**
 * One skill of the agent: switched on or off as flipped, opened to read or
 * edit, and deleted only after the row says which one.
 */
export function SkillRow(props: RowProps) {
  const { skill, editable, agentId, busy, order, onOpen } = props;
  const [deleting, setDeleting] = useState(false);
  return (
    <div className={settingsStyles.controlRow}>
      <div className={settingsStyles.rowCopy}>
        <strong>{skill.name}</strong>
        <span>{skill.description}</span>
        {deleting && <DeleteQuestion {...props} onDone={() => setDeleting(false)} />}
      </div>
      <div className={styles.row}>
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
    </div>
  );
}
