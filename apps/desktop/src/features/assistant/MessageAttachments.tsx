import { useState } from "react";
import { openAttachment } from "../../bridge/attachments";
import type { Attachment } from "../../bridge/generated/Attachment";
import { refusalOf } from "../../bridge/refusal";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import { fileSize } from "../../i18n/es/attachments";
import { attachmentFailure } from "./attachment-drafts";
import { useAttachmentUrl, useGallery } from "./AttachmentGallery";
import styles from "./Attachments.module.css";

/** An image the message carried, small; a click opens it in the viewer. */
function Thumbnail({ attachment }: { attachment: Attachment }) {
  const { view } = useGallery();
  const loaded = useAttachmentUrl(attachment);
  if (loaded.state === "failed")
    return (
      <p className={styles.failure} role="alert">
        {loaded.text}
      </p>
    );
  return (
    <button
      aria-label={es.attachments.viewImage(attachment.name)}
      className={styles.thumbnail}
      onClick={() => view(attachment)}
      type="button"
    >
      {loaded.state === "ready" && <img alt={attachment.name} src={loaded.url} />}
    </button>
  );
}

/** A PDF or any other file: its facts, and «Ver» (PDF), «Abrir», «Mostrar en Finder». */
function FileChip({ attachment }: { attachment: Attachment }) {
  const { view } = useGallery();
  const [failure, setFailure] = useState<string | null>(null);
  const open = (reveal: boolean) => {
    setFailure(null);
    openAttachment(attachment.id, reveal).catch((error: unknown) =>
      setFailure(attachmentFailure(refusalOf(error), attachment.name)),
    );
  };
  const facts = [fileSize(attachment.bytes)];
  if (attachment.pages !== undefined) facts.unshift(es.attachments.pages(attachment.pages));
  return (
    <div aria-label={attachment.name} className={styles.file} role="group">
      <span className={styles.fileName}>{attachment.name}</span>
      <span className={styles.fileFacts}>{facts.join(" · ")}</span>
      <span className={styles.fileActions}>
        {attachment.kind === "pdf" && (
          <Button onClick={() => view(attachment)} size={26} variant="ghost">
            {es.attachments.view}
          </Button>
        )}
        <Button onClick={() => open(false)} size={26} variant="ghost">
          {es.attachments.open}
        </Button>
        <Button onClick={() => open(true)} size={26} variant="ghost">
          {es.attachments.reveal}
        </Button>
      </span>
      {failure !== null && (
        <p className={styles.failure} role="alert">
          {failure}
        </p>
      )}
    </div>
  );
}

/** What a message of the user carried: images first, then the other files. */
export function MessageAttachments({ attachments }: { attachments: Attachment[] }) {
  const images = attachments.filter((attachment) => attachment.kind === "image");
  const files = attachments.filter((attachment) => attachment.kind !== "image");
  return (
    <div className={styles.attachments}>
      {images.length > 0 && (
        <div className={styles.thumbnails}>
          {images.map((attachment) => (
            <Thumbnail attachment={attachment} key={attachment.id} />
          ))}
        </div>
      )}
      {files.map((attachment) => (
        <FileChip attachment={attachment} key={attachment.id} />
      ))}
    </div>
  );
}
