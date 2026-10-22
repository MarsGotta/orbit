import { useEffect, useId, useState, type KeyboardEvent } from "react";
import type { Attachment } from "../../bridge/generated/Attachment";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { Loading } from "../../design/Spinner";
import { useModalFocus } from "../../design/use-modal-focus";
import { es } from "../../i18n/es";
import { useAttachmentUrl } from "./AttachmentGallery";
import { CopyImage, OpenButtons } from "./ViewerActions";
import { fileFacts } from "./MessageAttachments";
import styles from "./Viewer.module.css";

const kindName = (attachment: Attachment) =>
  attachment.kind === "image"
    ? es.attachments.image
    : attachment.kind === "pdf"
      ? es.attachments.pdf
      : es.attachments.file;

/** The image large, or the PDF in WebKit's own viewer; «Cargando…» until it arrives. */
function Stage({
  attachment,
  onBytes,
}: {
  attachment: Attachment;
  onBytes: (bytes: ArrayBuffer | null) => void;
}) {
  const pdf = attachment.kind === "pdf";
  const loaded = useAttachmentUrl(attachment, pdf ? "application/pdf" : attachment.mime);
  const bytes = loaded.state === "ready" ? loaded.bytes : null;
  useEffect(() => onBytes(bytes), [bytes, onBytes]);
  if (loaded.state === "loading") return <Loading>{es.attachments.loading}</Loading>;
  if (loaded.state === "failed")
    return (
      <p className={styles.failure} role="alert">
        {loaded.text}
      </p>
    );
  if (pdf) return <iframe className={styles.pdf} src={loaded.url} title={attachment.name} />;
  return <img alt={attachment.name} className={styles.image} src={loaded.url} />;
}

/** ← or → beside the attachment, still at the first and the last. */
function Arrow({
  to,
  onMove,
  side,
}: {
  to: number | null;
  onMove: (index: number) => void;
  side: "previous" | "next";
}) {
  const label = side === "previous" ? es.attachments.previous : es.attachments.next;
  return (
    <button
      aria-label={label}
      className={styles.arrow}
      disabled={to === null}
      onClick={() => to !== null && onMove(to)}
      title={`${label} (${side === "previous" ? "←" : "→"})`}
      type="button"
    >
      <Icon name={side} size={18} />
    </button>
  );
}

/** A dot per attachment of the conversation; the one shown, filled. */
function Dots({
  items,
  index,
  onMove,
}: {
  items: Attachment[];
  index: number;
  onMove: (index: number) => void;
}) {
  return (
    <nav aria-label={es.attachments.all} className={styles.dots}>
      {items.map((item, position) => (
        <button
          aria-current={position === index}
          aria-label={item.name}
          className={styles.dot}
          key={item.id}
          onClick={() => onMove(position)}
          type="button"
        />
      ))}
    </nav>
  );
}

/** The name, kind, size and place; «Copiar imagen», «Abrir», «Mostrar en Finder», «Cerrar». */
function ViewerHeader({
  attachment,
  bytes,
  onClose,
  position,
  titleId,
}: {
  attachment: Attachment;
  bytes: ArrayBuffer | null;
  onClose: () => void;
  position: string;
  titleId: string;
}) {
  return (
    <header className={styles.header}>
      <div className={styles.title}>
        <h2 id={titleId}>{attachment.name}</h2>
        <span>
          {kindName(attachment)} · {fileFacts(attachment)} · {position}
        </span>
      </div>
      <div className={styles.actions}>
        {attachment.kind === "image" && (
          <CopyImage bytes={bytes} key={attachment.id} mime={attachment.mime} />
        )}
        <OpenButtons attachment={attachment} key={`open-${attachment.id}`} />
        <Button className={styles.close} onClick={onClose} size={32} variant="ghost">
          {es.attachments.close}
          <span aria-hidden="true" className={styles.hint}>
            {es.attachments.escHint}
          </span>
        </Button>
      </div>
    </header>
  );
}

/**
 * An attachment at full screen over the conversation: its name, kind, size
 * and place among the conversation's («1 de 3»), what can be done with it,
 * arrows (and ← →) and dots to move, Esc or «Cerrar» to close.
 */
export function AttachmentViewer({
  index,
  items,
  onClose,
  onMove,
}: {
  index: number;
  items: Attachment[];
  onClose: () => void;
  onMove: (index: number) => void;
}) {
  const panel = useModalFocus(true);
  const titleId = useId();
  const [bytes, setBytes] = useState<ArrayBuffer | null>(null);
  const attachment = items[index];
  if (attachment === undefined) return null;
  const previous = index > 0 ? index - 1 : null;
  const next = index < items.length - 1 ? index + 1 : null;
  const keyDown = (event: KeyboardEvent) => {
    if (event.key === "Escape") {
      event.stopPropagation();
      onClose();
    } else if (event.key === "ArrowLeft" && previous !== null) onMove(previous);
    else if (event.key === "ArrowRight" && next !== null) onMove(next);
  };
  const several = items.length > 1;
  return (
    <section
      aria-labelledby={titleId}
      aria-modal="true"
      className={styles.viewer}
      onKeyDown={keyDown}
      ref={panel}
      role="dialog"
    >
      <ViewerHeader
        attachment={attachment}
        bytes={bytes}
        onClose={onClose}
        position={es.attachments.position(index + 1, items.length)}
        titleId={titleId}
      />
      <div className={styles.body}>
        {several && <Arrow onMove={onMove} side="previous" to={previous} />}
        <div className={styles.stage}>
          <Stage attachment={attachment} key={attachment.id} onBytes={setBytes} />
        </div>
        {several && <Arrow onMove={onMove} side="next" to={next} />}
      </div>
      {several && <Dots index={index} items={items} onMove={onMove} />}
    </section>
  );
}
