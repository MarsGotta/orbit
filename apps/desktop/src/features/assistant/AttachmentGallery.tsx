import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { attachmentBytes } from "../../bridge/attachments";
import type { Attachment } from "../../bridge/generated/Attachment";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import { refusalOf } from "../../bridge/refusal";
import { attachmentFailure } from "./attachment-drafts";
import { AttachmentViewer } from "./AttachmentViewer";

interface Gallery {
  /** An attachment's bytes, asked of the daemon once per conversation shown. */
  bytes: (attachmentId: string) => Promise<ArrayBuffer>;
  /** Opens the viewer on an image (among the conversation's) or a PDF. */
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
 * The attachments of the messages shown: their bytes, loaded once, and the
 * viewer, which moves between the images of those messages.
 */
export function AttachmentGallery({
  children,
  messages,
}: {
  children: ReactNode;
  messages: ChatMessage[];
}) {
  const cache = useRef(new Map<string, Promise<ArrayBuffer>>());
  const [open, setOpen] = useState<{ items: Attachment[]; index: number } | null>(null);
  const images = useMemo(
    () =>
      messages.flatMap((message) => message.attachments ?? []).filter((a) => a.kind === "image"),
    [messages],
  );
  const bytes = useCallback((attachmentId: string) => {
    const known = cache.current.get(attachmentId);
    if (known !== undefined) return known;
    const loading = attachmentBytes(attachmentId);
    cache.current.set(attachmentId, loading);
    // A failure is not kept: opening it again asks again.
    loading.catch(() => cache.current.delete(attachmentId));
    return loading;
  }, []);
  const gallery = useMemo<Gallery>(
    () => ({
      bytes,
      view: (attachment) => {
        const index = images.findIndex((image) => image.id === attachment.id);
        setOpen(index >= 0 ? { items: images, index } : { items: [attachment], index: 0 });
      },
    }),
    [bytes, images],
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
