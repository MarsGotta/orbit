import { MarkdownEditor } from "../../editor/MarkdownEditor";
import styles from "./Agents.module.css";

interface PersonalityEditorProps {
  /** Accessible name of the text box. */
  label: string;
  markdown: string;
  /** `false` when the agent's kind does not let Orbit change it. */
  editable: boolean;
  /** The Markdown as the user left it; called only when they edit. */
  onChange: (markdown: string) => void;
}

/**
 * An agent's personality in the notes' Markdown editor: its code blocks,
 * quotes and tables stay what they are, and what is sent is the Markdown as
 * typed, only once the user has changed something.
 */
export default function PersonalityEditor({
  editable,
  label,
  markdown,
  onChange,
}: PersonalityEditorProps) {
  return (
    <div className={styles.personalityFrame}>
      <MarkdownEditor label={label} onChange={onChange} readOnly={!editable} value={markdown} />
    </div>
  );
}
