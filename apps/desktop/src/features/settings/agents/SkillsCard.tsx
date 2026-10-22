import { useState } from "react";
import type { SkillsConfig } from "../../../bridge/generated/SkillsConfig";
import settingsStyles from "../SettingsView.module.css";
import styles from "./Agents.module.css";
import { NewSkill } from "./NewSkill";
import { copy, FeedbackLine, NotEditable, type OrderProps, Panel } from "./parts";
import { type OpenSkill, SkillEditor } from "./SkillEditor";
import { SkillRow } from "./SkillRow";
import type { useAgentConfig } from "./use-agent-config";

type Config = ReturnType<typeof useAgentConfig>;

/** The agent's skills: each one switched, opened or deleted, and new ones added. */
export function SkillsCard({
  skills,
  read,
  say,
  ...props
}: OrderProps & { skills: SkillsConfig; read: Config["skill"]; say: Config["say"] }) {
  const [open, setOpen] = useState<OpenSkill | null>(null);
  const editable = skills.editable;
  async function openSkill(name: string) {
    const skill = await read(name);
    if (skill) setOpen({ mode: "existing", name, content: skill.content, readOnly: !editable });
  }
  return (
    <Panel>
      <h3>{copy.skills}</h3>
      <p className={styles.hint}>{copy.skillCount(skills.items.length, skills.max_skills)}</p>
      <NotEditable editable={editable} cause={skills.cause} />
      {skills.items.length === 0 && <p className={styles.hint}>{copy.noSkills}</p>}
      <div className={settingsStyles.controls}>
        {skills.items.map((skill) => (
          <SkillRow
            editable={editable}
            key={skill.name}
            onOpen={(name) => void openSkill(name)}
            skill={skill}
            {...props}
          />
        ))}
      </div>
      {open && (
        <SkillEditor
          busy={props.busy}
          key={open.mode === "existing" ? `skill:${open.name}` : `new:${open.content}`}
          maxBytes={skills.max_skill_bytes}
          onClose={() => setOpen(null)}
          onSave={(name, content) =>
            void props
              .order({ call: "skill_save", agent_id: props.agentId, name, content }, "skills")
              .then((taken) => taken && setOpen(null))
          }
          skill={open}
        />
      )}
      {editable && (
        <NewSkill
          busy={props.busy}
          maxBytes={skills.max_skill_bytes}
          onCause={(text) => say("skills", text)}
          onOpen={(content) => setOpen({ mode: "new", content })}
        />
      )}
      <FeedbackLine feedback={props.feedback} place="skills" />
    </Panel>
  );
}
