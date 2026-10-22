import { useEffect, useRef, useState } from "react";
import { Popover } from "../../../design/Popover";
import styles from "./Agents.module.css";
import { copy } from "./parts";
import { folderSkill } from "./skill-file";

/** The two ways to start a skill, in a menu anchored to «Nueva habilidad». */
function NewSkillMenu({
  busy,
  onImport,
  onScratch,
}: {
  busy: boolean;
  onImport: () => void;
  onScratch: () => void;
}) {
  const [open, setOpen] = useState(false);
  const choose = (action: () => void) => {
    setOpen(false);
    action();
  };
  return (
    <Popover
      label={copy.newSkillMenu}
      onOpenChange={(next) => setOpen(next && !busy)}
      open={open}
      trigger={copy.newSkill}
      triggerClassName={styles.trigger}
    >
      <div className={styles.menu}>
        <button className={styles.option} onClick={() => choose(onScratch)} type="button">
          <span className={styles.optionTitle}>{copy.fromScratch}</span>
          <span className={styles.optionHelp}>{copy.fromScratchHelp}</span>
        </button>
        <button className={styles.option} onClick={() => choose(onImport)} type="button">
          <span className={styles.optionTitle}>{copy.importFolder}</span>
          <span className={styles.optionHelp}>{copy.importFolderHelp}</span>
        </button>
      </div>
    </Popover>
  );
}

/**
 * «Nueva habilidad»: written from scratch (from the template) or imported
 * from a folder chosen in the macOS panel WebKit shows. Only that folder's
 * `SKILL.md` is read, and only when it fits the hub's ceiling; either way
 * it opens in the editor before anything is sent.
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
    <div className={styles.add}>
      <NewSkillMenu
        busy={busy}
        onImport={() => folder.current?.click()}
        onScratch={() => onOpen(copy.template)}
      />
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
  );
}
