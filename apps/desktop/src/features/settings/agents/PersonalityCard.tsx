import { useState } from "react";
import type { PersonalityConfig } from "../../../bridge/generated/PersonalityConfig";
import { es } from "../../../i18n/es";
import { asTextOver, useLocalDraft } from "../../../state/local-drafts";
import { Button } from "../../../design/Button";
import type { useFeedback } from "../../../design/feedback";
import { Group } from "../layout";
import styles from "./Agents.module.css";
import { ByteCount, copy, FeedbackLine, NotEditable, type OrderProps } from "./parts";
import PersonalityEditor from "./PersonalityEditor";
import { utf8Bytes } from "./skill-file";

/**
 * The personality being written: `null` until the user edits, so the
 * editor's own Markdown of an untouched text is never sent back as a change.
 * Kept on this Mac until the hub takes it, over the text it was written on.
 */
function usePersonalityDraft(
  personality: PersonalityConfig,
  agentId: string,
  order: OrderProps["order"],
  saving: ReturnType<typeof useFeedback>,
) {
  const kept = useLocalDraft(
    personality.editable ? `agent.${agentId}.personality` : null,
    null,
    asTextOver,
  );
  const draft = kept.value?.from === personality.text ? kept.value.text : null;
  const [opened] = useState(() => draft ?? personality.text);
  const setDraft = (text: string) => kept.set({ from: personality.text, text });
  const save = () => {
    if (draft === null) return;
    const sent = draft;
    const taken = order({ call: "personality", agent_id: agentId, text: sent }, "personality");
    saving.track(
      taken.then((ok) => {
        if (!ok) return Promise.reject(new Error("not_taken"));
        // What was written after it left is still a draft.
        kept.set((now) => (now?.text === sent ? null : now));
        return undefined;
      }),
    );
  };
  return { draft, opened, recovered: kept.recovered && draft !== null, save, setDraft };
}

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
  const { draft, opened, recovered, save, setDraft } = usePersonalityDraft(
    personality,
    agentId,
    order,
    saving,
  );
  const bytes = utf8Bytes(draft ?? personality.text);
  const unchanged = draft === null || draft === personality.text;
  return (
    <Group label={copy.personality}>
      {!explained && <NotEditable cause={personality.cause} editable={personality.editable} />}
      <div className={styles.personality} data-readonly={!personality.editable || undefined}>
        <PersonalityEditor
          editable={personality.editable}
          label={copy.personalityLabel(name)}
          markdown={opened}
          onChange={setDraft}
        />
        {recovered && (
          <p className={styles.count} role="status">
            {es.controls.draftRecovered}
          </p>
        )}
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
