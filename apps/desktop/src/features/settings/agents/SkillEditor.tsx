import { Button } from "../../../design/Button";
import { Dialog } from "../../../design/Dialog";
import { es } from "../../../i18n/es";
import { asTextOver, useLocalDraft } from "../../../state/local-drafts";
import styles from "./Agents.module.css";
import { ByteCount, copy } from "./parts";
import { skillName, utf8Bytes } from "./skill-file";

/** A `SKILL.md` open to read or edit: one of the agent's, or a new one. */
export type OpenSkill =
  | { mode: "existing"; name: string; content: string; readOnly: boolean }
  | { mode: "new"; content: string };

/** «Cancelar» and «Guardar habilidad»; only «Cerrar» for one that opens to read. */
function EditorActions(props: {
  disabled: boolean;
  onClose: () => void;
  onSave: () => void;
  pending: boolean;
  readOnly: boolean;
}) {
  if (props.readOnly)
    return (
      <Button onClick={props.onClose} size={36}>
        {copy.close}
      </Button>
    );
  return (
    <>
      <Button onClick={props.onClose} size={36} variant="ghost">
        {es.controls.cancel}
      </Button>
      <Button
        disabled={props.disabled}
        feedback={props.pending ? "pending" : "idle"}
        onClick={props.onSave}
        size={36}
        variant="accent"
      >
        {copy.saveSkill}
      </Button>
    </>
  );
}

/** The name the frontmatter gives the skill, and its size against the hub's ceiling. */
function NameAndSize({
  bytes,
  maxBytes,
  name,
}: {
  bytes: number;
  maxBytes: number;
  name: string | null;
}) {
  return (
    <div className={styles.footer}>
      <span className={name === null ? styles.alert : styles.count}>
        {name !== null ? copy.name(name) : copy.noName}
      </span>
      <ByteCount bytes={bytes} max={maxBytes} />
    </div>
  );
}

/** A draft brought back from an earlier run, and the hub's refusal of the last save. */
function EditorLines({ recovered, refusal }: { recovered: boolean; refusal: string | null }) {
  return (
    <>
      {recovered && (
        <p className={styles.count} role="status">
          {es.controls.draftRecovered}
        </p>
      )}
      {refusal !== null && (
        <p className={styles.alert} role="alert">
          {refusal}
        </p>
      )}
    </>
  );
}

/**
 * What is written in the window, kept on this Mac over the text it opened
 * with until the hub takes it; closing the window on purpose lets it go.
 */
function useSkillDraft(
  draftKey: string | null,
  skill: OpenSkill,
  onSave: (name: string, content: string) => Promise<boolean>,
  onClose: () => void,
) {
  const kept = useLocalDraft(draftKey, null, asTextOver);
  const draft = kept.value?.from === skill.content ? kept.value.text : null;
  const content = draft ?? skill.content;
  return {
    close: () => {
      kept.set(null);
      onClose();
    },
    content,
    recovered: kept.recovered && draft !== null,
    save: (name: string) =>
      void onSave(name, content).then((taken) => {
        // What was written after it left is still a draft.
        if (taken) kept.set((now) => (now?.text === content ? null : now));
      }),
    setContent: (text: string) => kept.set({ from: skill.content, text }),
  };
}

/**
 * The whole `SKILL.md`, frontmatter included, as plain text in a window:
 * what is saved is exactly what is written. A new skill is named by its
 * frontmatter; one of a kind Orbit cannot change only opens to read.
 * Esc, a press outside or «Cancelar» close it and let its draft go; the
 * hub's refusal is said inside, and the window stays open with what was
 * written. Left by another way (another view, a quit, a fall), what was
 * written stays on this Mac and comes back when the same skill opens again.
 */
export function SkillEditor({
  skill,
  draftKey,
  maxBytes,
  busy,
  refusal,
  onSave,
  onClose,
}: {
  skill: OpenSkill;
  /** Where its draft is kept on this Mac; `null` for one that only opens to read. */
  draftKey: string | null;
  maxBytes: number;
  busy: boolean;
  /** Why the hub did not take the last save. */
  refusal: string | null;
  /** `true` once the hub took it. */
  onSave: (name: string, content: string) => Promise<boolean>;
  onClose: () => void;
}) {
  const draft = useSkillDraft(draftKey, skill, onSave, onClose);
  const { close, content } = draft;
  const name = skill.mode === "existing" ? skill.name : skillName(content);
  const readOnly = skill.mode === "existing" && skill.readOnly;
  const bytes = utf8Bytes(content);
  const unchanged = skill.mode === "existing" && content === skill.content;
  const footer = (
    <EditorActions
      disabled={busy || name === null || bytes > maxBytes || unchanged}
      onClose={close}
      onSave={() => name !== null && draft.save(name)}
      pending={busy}
      readOnly={readOnly}
    />
  );
  return (
    <Dialog
      footer={footer}
      onClose={close}
      subtitle={readOnly ? undefined : copy.editorHelp}
      title={skill.mode === "existing" ? skill.name : copy.editorNew}
    >
      <div className={styles.editor}>
        <textarea
          aria-label={copy.content}
          className={styles.content}
          onChange={(event) => draft.setContent(event.currentTarget.value)}
          readOnly={readOnly}
          rows={14}
          spellCheck={false}
          value={content}
        />
        <EditorLines recovered={draft.recovered} refusal={refusal} />
        {!readOnly && <NameAndSize bytes={bytes} maxBytes={maxBytes} name={name} />}
      </div>
    </Dialog>
  );
}
