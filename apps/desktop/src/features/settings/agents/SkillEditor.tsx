import { useState } from "react";
import { Button } from "../../../design/Button";
import styles from "./Agents.module.css";
import { ByteCount, copy } from "./parts";
import { skillName, utf8Bytes } from "./skill-file";

/** A `SKILL.md` open to read or edit: one of the agent's, or a new one. */
export type OpenSkill =
  | { mode: "existing"; name: string; content: string; readOnly: boolean }
  | { mode: "new"; content: string };

/**
 * The whole `SKILL.md`, frontmatter included, as plain text: what is saved
 * is exactly what is written. A new skill is named by its frontmatter.
 */
export function SkillEditor({
  skill,
  maxBytes,
  busy,
  onSave,
  onClose,
}: {
  skill: OpenSkill;
  maxBytes: number;
  busy: boolean;
  onSave: (name: string, content: string) => void;
  onClose: () => void;
}) {
  const [content, setContent] = useState(skill.content);
  const name = skill.mode === "existing" ? skill.name : skillName(content);
  const readOnly = skill.mode === "existing" && skill.readOnly;
  const bytes = utf8Bytes(content);
  const unchanged = skill.mode === "existing" && content === skill.content;
  return (
    <div className={styles.card}>
      <p className={styles.hint}>{name !== null ? copy.name(name) : copy.noName}</p>
      <textarea
        aria-label={copy.content}
        className={styles.content}
        onChange={(event) => setContent(event.currentTarget.value)}
        readOnly={readOnly}
        spellCheck={false}
        value={content}
      />
      <div className={styles.row}>
        {!readOnly && <ByteCount bytes={bytes} max={maxBytes} />}
        {!readOnly && (
          <Button
            disabled={busy || name === null || bytes > maxBytes || unchanged}
            onClick={() => name !== null && onSave(name, content)}
            variant="accent"
          >
            {copy.saveSkill}
          </Button>
        )}
        <Button onClick={onClose} variant="ghost">
          {copy.close}
        </Button>
      </div>
    </div>
  );
}
