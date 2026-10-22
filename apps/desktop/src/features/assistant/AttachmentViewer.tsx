import { useState, type KeyboardEvent } from "react";
import { copyImage } from "../../bridge/attachments";
import type { Attachment } from "../../bridge/generated/Attachment";
import { Button } from "../../design/Button";
import { useModalFocus } from "../../design/use-modal-focus";
import { es } from "../../i18n/es";
import { useAttachmentUrl, type LoadedAttachment } from "./AttachmentGallery";
import styles from "./Attachments.module.css";

type CopyState = { kind: "idle" } | { kind: "copied" } | { kind: "failed"; cause: string };

function CopyImage({ loaded, mime }: { loaded: LoadedAttachment; mime: string }) {
  const [state, setState] = useState<CopyState>({ kind: "idle" });
  if (loaded.state !== "ready") return null;
  const copy = () =>
    copyImage(loaded.bytes, mime).then(
      () => setState({ kind: "copied" }),
      (error: unknown) => setState({ kind: "failed", cause: String(error) }),
    );
  return (
    <>
      <Button onClick={() => void copy()} size={28}>
        {state.kind === "copied" ? es.attachments.copied : es.attachments.copy}
      </Button>
      {state.kind === "failed" && (
        <span role="alert">{es.chatActions.copyFailed(state.cause)}</span>
      )}
    </>
  );
}

/** The image large, or the PDF in WebKit's own viewer, with what can be done with it. */
function ViewerContent({ attachment }: { attachment: Attachment }) {
  const pdf = attachment.kind === "pdf";
  const loaded = useAttachmentUrl(attachment, pdf ? "application/pdf" : attachment.mime);
  let content;
  if (loaded.state === "loading") content = <p role="status">{es.attachments.loading}</p>;
  else if (loaded.state === "failed") content = <p role="alert">{loaded.text}</p>;
  else if (pdf)
    content = <iframe className={styles.viewerPdf} src={loaded.url} title={attachment.name} />;
  else content = <img alt={attachment.name} className={styles.viewerImage} src={loaded.url} />;
  return (
    <>
      <div className={styles.viewerStage}>{content}</div>
      {!pdf && <CopyImage loaded={loaded} mime={attachment.mime} />}
    </>
  );
}

/**
 * An attachment opened large. Between images, «Anterior» and «Siguiente»
 * (or ← and →) move through the conversation's; Esc or «Cerrar» closes it.
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
  return (
    <div className={styles.backdrop} role="presentation">
      <section
        aria-label={attachment.name}
        aria-modal="true"
        className={styles.viewer}
        onKeyDown={keyDown}
        ref={panel}
        role="dialog"
      >
        <header className={styles.viewerHeader}>
          <strong>{attachment.name}</strong>
          <Button onClick={onClose} size={28} variant="ghost">
            {es.attachments.close}
          </Button>
        </header>
        <ViewerContent attachment={attachment} key={attachment.id} />
        {items.length > 1 && (
          <footer className={styles.viewerNav}>
            <Button
              disabled={previous === null}
              onClick={() => previous !== null && onMove(previous)}
            >
              {es.attachments.previous}
            </Button>
            <Button disabled={next === null} onClick={() => next !== null && onMove(next)}>
              {es.attachments.next}
            </Button>
          </footer>
        )}
      </section>
    </div>
  );
}
