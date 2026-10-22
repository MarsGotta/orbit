import { useState } from "react";
import type { SkillsConfig } from "../../../bridge/generated/SkillsConfig";
import { Group, List } from "../layout";
import styles from "./Agents.module.css";
import { NewSkill } from "./NewSkill";
import { copy, FeedbackLine, NotEditable, type OrderProps } from "./parts";
import { type OpenSkill, SkillEditor } from "./SkillEditor";
import { SkillRow } from "./SkillRow";
import type { Feedback, useAgentConfig } from "./use-agent-config";

type Config = ReturnType<typeof useAgentConfig>;

/**
 * The skill open in the editor's window, and the hub's refusal to say in it:
 * only one given after it opened is that window's.
 */
function useSkillWindow(feedback: Feedback | null, read: Config["skill"], skills: SkillsConfig) {
  const [open, setOpenSkill] = useState<OpenSkill | null>(null);
  const [openedWith, setOpenedWith] = useState<Feedback | null>(null);
  const refusal =
    feedback !== openedWith && feedback?.place === "skills" && feedback.kind === "error"
      ? feedback.text
      : null;
  const setOpen = (skill: OpenSkill | null) => {
    setOpenedWith(feedback);
    setOpenSkill(skill);
  };
  async function openSkill(name: string) {
    const skill = await read(name);
    if (skill)
      setOpen({ mode: "existing", name, content: skill.content, readOnly: !skills.editable });
  }
  return { open, setOpen, refusal, openSkill };
}

/**
 * «Habilidades · N de 128»: each one switched, opened or deleted, and new
 * ones written from scratch or imported, in the editor's window.
 */
export function SkillsCard({
  skills,
  explained,
  read,
  say,
  ...props
}: OrderProps & {
  skills: SkillsConfig;
  explained: boolean;
  read: Config["skill"];
  say: Config["say"];
}) {
  const { open, setOpen, refusal, openSkill } = useSkillWindow(props.feedback, read, skills);
  async function save(name: string, content: string) {
    const call = { call: "skill_save", agent_id: props.agentId, name, content } as const;
    if (await props.order(call, "skills")) setOpen(null);
  }
  return (
    <Group count={copy.skillCount(skills.items.length, skills.max_skills)} label={copy.skills}>
      {!explained && <NotEditable cause={skills.cause} editable={skills.editable} />}
      <List>
        {skills.items.length === 0 && <p className={styles.empty}>{copy.noSkills}</p>}
        {skills.items.map((skill) => (
          <SkillRow
            editable={skills.editable}
            key={skill.name}
            onOpen={(name) => void openSkill(name)}
            skill={skill}
            {...props}
          />
        ))}
      </List>
      {skills.editable && (
        <NewSkill
          busy={props.busy}
          maxBytes={skills.max_skill_bytes}
          onCause={(text) => say("skills", text)}
          onOpen={(content) => setOpen({ mode: "new", content })}
        />
      )}
      {open === null ? (
        <FeedbackLine feedback={props.feedback} place="skills" />
      ) : (
        <SkillEditor
          busy={props.busy}
          key={open.mode === "existing" ? `skill:${open.name}` : `new:${open.content}`}
          maxBytes={skills.max_skill_bytes}
          onClose={() => setOpen(null)}
          onSave={(name, content) => void save(name, content)}
          refusal={refusal}
          skill={open}
        />
      )}
    </Group>
  );
}
