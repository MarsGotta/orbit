import { useCopy } from "../../i18n/CopyProvider";
import { useState } from "react";
import { copyImage, openAttachment } from "../../bridge/attachments";
import type { Attachment } from "../../bridge/generated/Attachment";
import { refusalOf } from "../../bridge/refusal";
import { Button } from "../../design/Button";
import { useFeedback } from "../../design/feedback";
import { useTrack } from "../../state/use-in-flight";

import { attachmentFailure } from "./attachment-drafts";
import styles from "./Viewer.module.css";

/** «Copiar imagen», which says «✓ Copiada» once the clipboard has it. */
export function CopyImage({ bytes, mime }: { bytes: ArrayBuffer | null; mime: string }) {
  const es = useCopy();
  const { feedback, track } = useFeedback();
  const [failure, setFailure] = useState<string | null>(null);
  return (
    <>
      {failure !== null && (
        <span className={styles.failure} role="alert">
          {failure}
        </span>
      )}
      <Button
        disabled={bytes === null}
        doneLabel={es.attachments.copiedImage}
        feedback={feedback}
        onClick={() => {
          if (bytes === null) return;
          setFailure(null);
          const copying = copyImage(bytes, mime);
          copying.catch((error: unknown) => setFailure(es.chatActions.copyFailed(String(error))));
          track(copying);
        }}
        pendingLabel={es.attachments.copying}
        size={32}
        variant="ghost"
      >
        {es.attachments.copyImage}
      </Button>
    </>
  );
}

/** «Abrir» and «Mostrar en Finder», each saying «Abriendo…» until macOS has done it. */
export function OpenButtons({ attachment }: { attachment: Attachment }) {
  const es = useCopy();
  const track = useTrack();
  const [opening, setOpening] = useState<boolean | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const open = (reveal: boolean) => {
    setFailure(null);
    setOpening(reveal);
    track("attachment_open", "assistant", () => openAttachment(attachment.id, reveal), "assistant")
      .catch((error: unknown) => setFailure(attachmentFailure(refusalOf(error), attachment.name)))
      .finally(() => setOpening(null));
  };
  return (
    <>
      {failure !== null && (
        <span className={styles.failure} role="alert">
          {failure}
        </span>
      )}
      <span aria-live="polite" className={styles.openButtons}>
        <Button disabled={opening !== null} onClick={() => open(false)} size={32} variant="ghost">
          {opening === false ? es.attachments.opening : es.attachments.open}
        </Button>
        <Button disabled={opening !== null} onClick={() => open(true)} size={32} variant="ghost">
          {opening === true ? es.attachments.opening : es.attachments.reveal}
        </Button>
      </span>
    </>
  );
}
