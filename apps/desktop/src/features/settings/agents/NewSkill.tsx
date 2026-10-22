import { useEffect, useRef } from "react";
import { Button } from "../../../design/Button";
import styles from "./Agents.module.css";
import { copy } from "./parts";
import { folderSkill } from "./skill-file";

/**
 * A new skill: from the template, pasted, or the `SKILL.md` of a folder
 * chosen in the macOS panel WebKit shows. Only that file is read, and only
 * when it fits the hub's ceiling; it opens in the editor before it is sent.
 */
export function NewSkill({
  busy,
  maxBytes,
  onOpen,
  onCause,
}: {
  busy: boolean;
  maxBytes: number;
  onOpen: (content: string) => void;
  onCause: (text: string) => void;
}) {
  const folder = useRef<HTMLInputElement>(null);
  useEffect(() => {
    // WebKit's panel chooses folders only with this attribute, which
    // React's input props do not name.
    folder.current?.setAttribute("webkitdirectory", "");
  }, []);

  async function importFolder(files: File[]) {
    const { folder: name, file } = folderSkill(files);
    if (!file) return onCause(copy.noSkillFile(name));
    if (file.size > maxBytes) return onCause(copy.skillFileTooLarge(name, file.size, maxBytes));
    try {
      onOpen(await file.text());
    } catch {
      onCause(copy.skillFileUnreadable(name));
    }
  }

  return (
    <>
      <p className={styles.hint}>{copy.importHelp}</p>
      <div className={styles.row}>
        <Button disabled={busy} onClick={() => onOpen(copy.template)}>
          {copy.newSkill}
        </Button>
        <Button disabled={busy} onClick={() => onOpen("")}>
          {copy.paste}
        </Button>
        <Button disabled={busy} onClick={() => folder.current?.click()}>
          {copy.importFolder}
        </Button>
        <input
          aria-label={copy.folderInput}
          hidden
          onChange={(event) => {
            const files = Array.from(event.currentTarget.files ?? []);
            event.currentTarget.value = "";
            if (files.length > 0) void importFolder(files);
          }}
          ref={folder}
          type="file"
        />
      </div>
    </>
  );
}
