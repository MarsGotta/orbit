import { getCopy } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import { useEffect, useRef, useState } from "react";
import { openAttachment } from "../../bridge/attachments";
import { useTrack } from "../../state/use-in-flight";
import type { Attachment } from "../../bridge/generated/Attachment";
import { refusalOf } from "../../bridge/refusal";
import { Button } from "../../design/Button";

import { attachmentFailure } from "./attachment-drafts";
import { useAttachmentUrl, useGallery } from "./AttachmentGallery";
import styles from "./Attachments.module.css";

/** A preview can open the original even before its nearby bytes have arrived. */
function ThumbnailButton({ attachment, url }: { attachment: Attachment; url?: string }) {
  const es = useCopy();
  const { view } = useGallery();
  return (
    <button
      aria-label={es.attachments.viewImage(attachment.name)}
      className={styles.thumbnail}
      onClick={() => view(attachment)}
      type="button"
    >
      {url !== undefined && <img alt={attachment.name} src={url} />}
    </button>
  );
}

/** Byte and blob ownership ends when the thumbnail leaves the viewport margin. */
function ThumbnailContent({ attachment }: { attachment: Attachment }) {
  const loaded = useAttachmentUrl(attachment);
  if (loaded.state === "failed")
    return (
      <p className={styles.failure} role="alert">
        {loaded.text}
      </p>
    );
  return (
    <ThumbnailButton
      attachment={attachment}
      url={loaded.state === "ready" ? loaded.url : undefined}
    />
  );
}

/** The observed slot stays mounted so a failed preview can show its full explanation. */
function Thumbnail({ attachment }: { attachment: Attachment }) {
  const element = useRef<HTMLDivElement>(null);
  const [near, setNear] = useState(false);
  useEffect(() => {
    // 100 px of prefetch headroom covers nearby thumbnails without loading the whole history.
    const observer = new IntersectionObserver(
      ([entry]) => {
        setNear(entry?.isIntersecting ?? false);
      },
      { rootMargin: "100px" },
    );
    if (element.current) observer.observe(element.current);
    return () => observer.disconnect();
  }, []);
  return (
    <div ref={element}>
      {near ? (
        <ThumbnailContent attachment={attachment} />
      ) : (
        <ThumbnailButton attachment={attachment} />
      )}
    </div>
  );
}

/** «2 páginas · 1,5 MB»: what the chip and the viewer say of a file. */
export function fileFacts(attachment: Attachment): string {
  const facts = [getCopy().extras.fileSize(attachment.bytes)];
  if (attachment.pages !== undefined) facts.unshift(getCopy().attachments.pages(attachment.pages));
  return facts.join(" · ");
}

/** A PDF, as a chip that opens it in the viewer. */
function PdfChip({ attachment }: { attachment: Attachment }) {
  const es = useCopy();
  const { view } = useGallery();
  return (
    <button
      aria-label={es.attachments.viewImage(attachment.name)}
      className={styles.fileChip}
      onClick={() => view(attachment)}
      type="button"
    >
      <span className={styles.fileName}>{attachment.name}</span>
      <span className={styles.fileFacts}>· {fileFacts(attachment)}</span>
    </button>
  );
}

/** Any other file: its facts, «Abrir» and «Mostrar en Finder». */
function FileChip({ attachment }: { attachment: Attachment }) {
  const es = useCopy();
  const track = useTrack();
  const [failure, setFailure] = useState<string | null>(null);
  const open = (reveal: boolean) => {
    setFailure(null);
    track(
      "attachment_open",
      "assistant",
      () => openAttachment(attachment.id, reveal),
      "assistant",
    ).catch((error: unknown) => setFailure(attachmentFailure(refusalOf(error), attachment.name)));
  };
  return (
    <div aria-label={attachment.name} className={styles.file} role="group">
      <span className={styles.fileName}>{attachment.name}</span>
      <span className={styles.fileFacts}>{fileFacts(attachment)}</span>
      <span className={styles.fileActions}>
        <Button onClick={() => open(false)} size={28} variant="ghost">
          {es.attachments.open}
        </Button>
        <Button onClick={() => open(true)} size={28} variant="ghost">
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

/** What a message of the user carried, above its words: images, PDFs, other files. */
export function MessageAttachments({ attachments }: { attachments: Attachment[] }) {
  const images = attachments.filter((attachment) => attachment.kind === "image");
  const pdfs = attachments.filter((attachment) => attachment.kind === "pdf");
  const files = attachments.filter(
    (attachment) => attachment.kind !== "image" && attachment.kind !== "pdf",
  );
  return (
    <div className={styles.attachments}>
      {images.length > 0 && (
        <div className={styles.thumbnails}>
          {images.map((attachment) => (
            <Thumbnail attachment={attachment} key={attachment.id} />
          ))}
        </div>
      )}
      {pdfs.map((attachment) => (
        <PdfChip attachment={attachment} key={attachment.id} />
      ))}
      {files.map((attachment) => (
        <FileChip attachment={attachment} key={attachment.id} />
      ))}
    </div>
  );
}
