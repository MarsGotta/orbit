import { useState } from "react";
import { Button } from "../../../design/Button";
import { Dialog } from "../../../design/Dialog";
import { es } from "../../../i18n/es";
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

/**
 * The whole `SKILL.md`, frontmatter included, as plain text in a window:
 * what is saved is exactly what is written. A new skill is named by its
 * frontmatter; one of a kind Orbit cannot change only opens to read.
 * Esc, a press outside or «Cancelar» close it; the hub's refusal is said
 * inside, and the window stays open with what was written.
 */
export function SkillEditor({
  skill,
  maxBytes,
  busy,
  refusal,
  onSave,
  onClose,
}: {
  skill: OpenSkill;
  maxBytes: number;
  busy: boolean;
  /** Why the hub did not take the last save. */
  refusal: string | null;
  onSave: (name: string, content: string) => void;
  onClose: () => void;
}) {
  const [content, setContent] = useState(skill.content);
  const name = skill.mode === "existing" ? skill.name : skillName(content);
  const readOnly = skill.mode === "existing" && skill.readOnly;
  const bytes = utf8Bytes(content);
  const unchanged = skill.mode === "existing" && content === skill.content;
  const footer = (
    <EditorActions
      disabled={busy || name === null || bytes > maxBytes || unchanged}
      onClose={onClose}
      onSave={() => name !== null && onSave(name, content)}
      pending={busy}
      readOnly={readOnly}
    />
  );
  return (
    <Dialog
      footer={footer}
      onClose={onClose}
      subtitle={readOnly ? undefined : copy.editorHelp}
      title={skill.mode === "existing" ? skill.name : copy.editorNew}
    >
      <div className={styles.editor}>
        <textarea
          aria-label={copy.content}
          className={styles.content}
          onChange={(event) => setContent(event.currentTarget.value)}
          readOnly={readOnly}
          rows={14}
          spellCheck={false}
          value={content}
        />
        {refusal !== null && (
          <p className={styles.alert} role="alert">
            {refusal}
          </p>
        )}
        {!readOnly && <NameAndSize bytes={bytes} maxBytes={maxBytes} name={name} />}
      </div>
    </Dialog>
  );
}
