import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { attachmentBytes } from "../../bridge/attachments";
import type { Attachment } from "../../bridge/generated/Attachment";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import { refusalOf } from "../../bridge/refusal";
import { useTrack } from "../../state/use-in-flight";
import { attachmentFailure } from "./attachment-drafts";
import { AttachmentViewer } from "./AttachmentViewer";

interface Gallery {
  /** Original bytes, held only by their mounted thumbnail or viewer. */
  bytes: (attachmentId: string) => Promise<ArrayBuffer>;
  /** Opens the viewer on an image or a PDF, among the conversation's. */
  view: (attachment: Attachment) => void;
}

const GalleryContext = createContext<Gallery>({
  bytes: attachmentBytes,
  view: () => undefined,
});

/** An attachment as a blob URL of its bytes, freed when it is no longer shown. */
export type LoadedAttachment =
  | { state: "loading" }
  | { state: "ready"; url: string; bytes: ArrayBuffer }
  | { state: "failed"; text: string };

export function useAttachmentUrl(attachment: Attachment, mime = attachment.mime) {
  const { bytes } = useContext(GalleryContext);
  const [loaded, setLoaded] = useState<LoadedAttachment>({ state: "loading" });
  const { id, name } = attachment;
  useEffect(() => {
    setLoaded({ state: "loading" });
    let url: string | undefined;
    let gone = false;
    bytes(id).then(
      (data) => {
        if (gone) return;
        url = URL.createObjectURL(new Blob([data], { type: mime }));
        setLoaded({ state: "ready", url, bytes: data });
      },
      (error: unknown) => {
        if (!gone) setLoaded({ state: "failed", text: attachmentFailure(refusalOf(error), name) });
      },
    );
    return () => {
      gone = true;
      if (url !== undefined) URL.revokeObjectURL(url);
    };
  }, [bytes, id, mime, name]);
  return loaded;
}

export function useGallery(): Gallery {
  return useContext(GalleryContext);
}

/**
 * The viewer moves between the images and PDFs of the shown messages.
 * Successful full-file promises are not retained for the conversation.
 */
export function AttachmentGallery({
  children,
  messages,
}: {
  children: ReactNode;
  messages: ChatMessage[];
}) {
  const [open, setOpen] = useState<{ items: Attachment[]; index: number } | null>(null);
  const viewable = useMemo(
    () =>
      messages
        .flatMap((message) => message.attachments ?? [])
        .filter((a) => a.kind === "image" || a.kind === "pdf"),
    [messages],
  );
  const track = useTrack();
  const bytes = useCallback(
    (attachmentId: string) =>
      track("attachment_content", "assistant", () => attachmentBytes(attachmentId), "assistant"),
    [track],
  );
  const gallery = useMemo<Gallery>(
    () => ({
      bytes,
      view: (attachment) => {
        const index = viewable.findIndex((each) => each.id === attachment.id);
        setOpen(index >= 0 ? { items: viewable, index } : { items: [attachment], index: 0 });
      },
    }),
    [bytes, viewable],
  );
  return (
    <GalleryContext.Provider value={gallery}>
      {children}
      {open !== null && (
        <AttachmentViewer
          index={open.index}
          items={open.items}
          onClose={() => setOpen(null)}
          onMove={(index) => setOpen({ ...open, index })}
        />
      )}
    </GalleryContext.Provider>
  );
}
