import { useCallback, useEffect, useRef, useState, type ClipboardEvent } from "react";
import {
  addAttachment,
  chooseFiles,
  onFilesDropped,
  type AttachmentSource,
} from "../../bridge/attachments";
import type { AttachmentLimits } from "../../bridge/generated/AttachmentLimits";
import { refusalOf } from "../../bridge/refusal";
import { es } from "../../i18n/es";
import { localRefusal, settled, type DraftAttachment } from "./attachment-drafts";

/**
 * A paste longer than this becomes «Texto pegado.txt» instead of going into
 * the box. About a page and a half of prose: past it the box would hold a
 * block the user does not reread and that hides the conversation. Kept below
 * the hub's 32,000-character turn limit (`protocol.py`), so such a paste never
 * meets that refusal. Counted in UTF-16 units, never fewer than characters.
 * A judgement, not measured.
 */
export const PASTED_TEXT_ATTACHMENT_CHARS = 4_000;

function base64(bytes: Uint8Array): string {
  let binary = "";
  // Spread in slices: one call with millions of arguments overflows the stack.
  for (let start = 0; start < bytes.length; start += 0x8000)
    binary += String.fromCharCode(...bytes.subarray(start, start + 0x8000));
  return btoa(binary);
}

function readBytes(file: Blob): Promise<Uint8Array> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(new Uint8Array(reader.result as ArrayBuffer));
    reader.onerror = () => reject(reader.error ?? new Error("FileReader failed"));
    reader.readAsArrayBuffer(file);
  });
}

function baseName(path: string): string {
  return path.split("/").pop() ?? path;
}

/** Files dropped onto the window while the composer is shown, by path. */
function useFileDrop(handler: (paths: string[]) => void) {
  const latest = useRef(handler);
  latest.current = handler;
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let gone = false;
    onFilesDropped((paths) => latest.current(paths)).then(
      (stop) => (gone ? stop() : (unlisten = stop)),
      () => undefined, // Outside the app's webview there is nothing to drop on.
    );
    return () => {
      gone = true;
      unlisten?.();
    };
  }, []);
}

/**
 * A pasted image becomes an attachment of its bytes, and a long text one
 * named «Texto pegado.txt»; anything else is pasted into the box as usual.
 */
function pasteInto(
  event: ClipboardEvent<HTMLTextAreaElement>,
  supported: boolean,
  addBytes: (name: string, bytes: Uint8Array) => void,
) {
  const images = Array.from(event.clipboardData.files).filter((file) =>
    file.type.startsWith("image/"),
  );
  if (images.length > 0) {
    event.preventDefault();
    for (const image of images) {
      const name = image.name !== "" ? image.name : es.attachments.pastedImage;
      void readBytes(image).then((bytes) => addBytes(name, bytes));
    }
    return;
  }
  // Without attachments a long text stays text: the box takes it as always.
  if (!supported) return;
  const text = event.clipboardData.getData("text/plain");
  if (text.length <= PASTED_TEXT_ATTACHMENT_CHARS) return;
  event.preventDefault();
  addBytes(es.attachments.pastedText, new TextEncoder().encode(text));
}

/** The chips of the next message: what is uploading, ready or refused. */
export function useAttachmentDrafts(limits: AttachmentLimits | undefined) {
  const [drafts, setDrafts] = useState<DraftAttachment[]>([]);
  // Several files arrive at once; each is checked against the ones before it.
  const current = useRef<DraftAttachment[]>([]);
  const nextKey = useRef(0);
  const update = useCallback((change: (drafts: DraftAttachment[]) => DraftAttachment[]) => {
    current.current = change(current.current);
    setDrafts(current.current);
  }, []);

  const upload = (name: string, source: AttachmentSource, bytes?: number) => {
    const key = nextKey.current++;
    const refused = localRefusal(limits, current.current, name, bytes);
    if (refused !== null) {
      update((all) => [...all, { key, state: "failed", name, refusal: refused }]);
      return;
    }
    update((all) => [...all, { key, state: "uploading", name, bytes }]);
    addAttachment(source).then(
      (added) => update((all) => settled(limits, all, key, added, () => nextKey.current++)),
      (error: unknown) =>
        update((all) =>
          all.map((each) =>
            each.key === key ? { key, state: "failed", name, refusal: refusalOf(error) } : each,
          ),
        ),
    );
  };

  const addPaths = (paths: string[]) => {
    for (const path of paths) upload(baseName(path), { from: "path", path });
  };
  const addBytes = (name: string, bytes: Uint8Array) =>
    upload(name, { from: "bytes", name, data_base64: base64(bytes) }, bytes.length);

  useFileDrop(addPaths);

  return {
    drafts,
    supported: limits !== undefined,
    ready: drafts.flatMap((draft) => (draft.state === "ready" ? [draft.attachment] : [])),
    uploading: drafts.some((draft) => draft.state === "uploading"),
    choose: () => void chooseFiles().then(addPaths),
    addWindow: () => upload(es.attachments.window, { from: "window" }),
    paste: (event: ClipboardEvent<HTMLTextAreaElement>) =>
      pasteInto(event, limits !== undefined, addBytes),
    remove: (key: number) => update((all) => all.filter((draft) => draft.key !== key)),
    clear: () => update(() => []),
  };
}
