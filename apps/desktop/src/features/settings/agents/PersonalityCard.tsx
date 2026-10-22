import { useState } from "react";
import type { PersonalityConfig } from "../../../bridge/generated/PersonalityConfig";
import { Button } from "../../../design/Button";
import type { useFeedback } from "../../../design/feedback";
import { Group } from "../layout";
import styles from "./Agents.module.css";
import { ByteCount, copy, FeedbackLine, NotEditable, type OrderProps } from "./parts";
import PersonalityEditor from "./PersonalityEditor";
import { utf8Bytes } from "./skill-file";

/**
 * The agent's personality in the Markdown editor, with its size against the
 * hub's ceiling («N de 16.384 bytes»). «Guardar» says «Guardando…» and then
 * «✓ Guardado en el hub»; a refusal is said below it.
 */
export function PersonalityCard({
  name,
  personality,
  explained,
  saving,
  agentId,
  busy,
  feedback,
  order,
}: OrderProps & {
  name: string;
  personality: PersonalityConfig;
  explained: boolean;
  /** Held by the panel: a saved text remounts this card before the hub's answer is told. */
  saving: ReturnType<typeof useFeedback>;
}) {
  // `null` until the user edits: the editor's own Markdown of an untouched
  // text is never sent back as a change.
  const [draft, setDraft] = useState<string | null>(null);
  const bytes = utf8Bytes(draft ?? personality.text);
  const unchanged = draft === null || draft === personality.text;
  const save = () => {
    if (draft === null) return;
    const taken = order({ call: "personality", agent_id: agentId, text: draft }, "personality");
    saving.track(taken.then((ok) => (ok ? undefined : Promise.reject(new Error("not_taken")))));
  };
  return (
    <Group label={copy.personality}>
      {!explained && <NotEditable cause={personality.cause} editable={personality.editable} />}
      <div className={styles.personality} data-readonly={!personality.editable || undefined}>
        <PersonalityEditor
          editable={personality.editable}
          label={copy.personalityLabel(name)}
          markdown={personality.text}
          onChange={setDraft}
        />
        <div className={styles.footer}>
          <ByteCount bytes={bytes} max={personality.max_bytes} />
          {personality.editable && (
            <Button
              disabled={busy || bytes > personality.max_bytes || unchanged}
              doneLabel={copy.saved}
              feedback={saving.feedback}
              onClick={save}
              size={28}
              variant="accent"
            >
              {copy.savePersonality}
            </Button>
          )}
        </div>
      </div>
      <FeedbackLine errorsOnly feedback={feedback} place="personality" />
    </Group>
  );
}
