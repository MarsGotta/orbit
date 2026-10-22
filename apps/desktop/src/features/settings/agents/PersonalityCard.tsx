import { useState } from "react";
import type { PersonalityConfig } from "../../../bridge/generated/PersonalityConfig";
import { Button } from "../../../design/Button";
import styles from "./Agents.module.css";
import { ByteCount, copy, FeedbackLine, NotEditable, type OrderProps, Panel } from "./parts";
import PersonalityEditor from "./PersonalityEditor";
import { utf8Bytes } from "./skill-file";

/** The agent's personality, in the Markdown editor, saved as the user leaves it. */
export function PersonalityCard({
  name,
  personality,
  agentId,
  busy,
  feedback,
  order,
}: OrderProps & { name: string; personality: PersonalityConfig }) {
  // `null` until the user edits: the editor's own Markdown of an untouched
  // text is never sent back as a change.
  const [draft, setDraft] = useState<string | null>(null);
  const bytes = utf8Bytes(draft ?? personality.text);
  return (
    <Panel>
      <h3>{copy.personality}</h3>
      <NotEditable editable={personality.editable} cause={personality.cause} />
      <PersonalityEditor
        editable={personality.editable}
        label={copy.personalityLabel(name)}
        markdown={personality.text}
        onChange={setDraft}
      />
      {personality.editable && (
        <div className={styles.row}>
          <ByteCount bytes={bytes} max={personality.max_bytes} />
          <Button
            disabled={
              busy || bytes > personality.max_bytes || draft === null || draft === personality.text
            }
            onClick={() =>
              draft !== null &&
              void order({ call: "personality", agent_id: agentId, text: draft }, "personality")
            }
            variant="accent"
          >
            {copy.savePersonality}
          </Button>
        </div>
      )}
      <FeedbackLine feedback={feedback} place="personality" />
    </Panel>
  );
}
