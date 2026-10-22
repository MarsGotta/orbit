import { es } from "../../i18n/es";
import { fileSize } from "../../i18n/es/attachments";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { attachmentFailure, type DraftAttachment } from "./attachment-drafts";
import styles from "./Attachments.module.css";

function detail(draft: DraftAttachment): string {
  if (draft.state === "uploading") return es.attachments.uploading;
  if (draft.state === "ready") {
    const size = fileSize(draft.attachment.bytes);
    return draft.note === undefined ? size : `${size} · ${draft.note}`;
  }
  return attachmentFailure(draft.refusal, draft.name);
}

/** The files of the next message, before sending: each can be removed. */
export function AttachmentChips({
  drafts,
  onRemove,
}: {
  drafts: DraftAttachment[];
  onRemove: (key: number) => void;
}) {
  if (drafts.length === 0) return null;
  return (
    <ul className={styles.chips}>
      {drafts.map((draft) => (
        <li
          aria-label={draft.name}
          className={styles.chip}
          data-state={draft.state}
          key={draft.key}
          role="group"
        >
          <span className={styles.chipName}>{draft.name}</span>
          <span className={styles.chipDetail} role={draft.state === "failed" ? "alert" : undefined}>
            {detail(draft)}
          </span>
          <button
            aria-label={es.attachments.remove(draft.name)}
            className={styles.chipRemove}
            onClick={() => onRemove(draft.key)}
            type="button"
          >
            <Icon name="close" size={10} />
          </button>
        </li>
      ))}
    </ul>
  );
}

/**
 * «Adjuntar» opens the file dialog; «Adjuntar ventana» sends the frontmost
 * window as an image. A hub without attachments offers neither, and says why.
 */
export function AttachButtons({
  disabled,
  onAttach,
  onAttachWindow,
  supported,
}: {
  disabled: boolean;
  onAttach: () => void;
  onAttachWindow: () => void;
  supported: boolean;
}) {
  const reason = supported ? undefined : es.attachments.unsupported;
  return (
    <>
      <Button
        aria-label={es.attachments.attach}
        disabled={disabled || !supported}
        onClick={onAttach}
        size={30}
        title={reason}
        variant="icon"
      >
        <Icon name="attach" size={14} />
      </Button>
      <Button
        aria-label={es.attachments.attachWindow}
        disabled={disabled || !supported}
        onClick={onAttachWindow}
        size={30}
        title={reason}
        variant="icon"
      >
        <Icon name="screen" size={14} />
      </Button>
    </>
  );
}
