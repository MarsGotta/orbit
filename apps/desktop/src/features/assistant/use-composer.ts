import { useCallback, useRef, useState, type KeyboardEvent } from "react";
import type { AttachmentLimits } from "../../bridge/generated/AttachmentLimits";
import type { ChatSnapshot } from "../../bridge/generated/ChatSnapshot";
import type { HubLinkConnection } from "../../bridge/generated/HubLinkConnection";
import type { useSpanish } from "../../i18n/AssistantCopy";
import { useVoiceNote, type VoiceSend } from "./useVoiceNote";
import { useComposerInsert } from "./composer-insert";
import { turnInFlight } from "./conversation";
import { useAttachmentDrafts } from "./useAttachmentDrafts";

export interface ComposerProps {
  /** The hub's attachment ceilings; absent, the hub has no attachments. */
  attachmentLimits?: AttachmentLimits;
  chat: ChatSnapshot;
  daemonReady: boolean;
  hub: HubLinkConnection;
  paired: boolean;
  send: VoiceSend;
}

type Copy = ReturnType<typeof useSpanish>;

/**
 * Why nothing can be sent now, or null when it can. Writing is never
 * stopped: what is written stays until it can be sent. Without pairing the
 * view itself says what to do, so the composer adds nothing.
 */
function blockedReason(props: ComposerProps, uploading: boolean, es: Copy): string | null {
  const { chat } = props;
  const blocked = es.assistant.blocked;
  if (!props.paired) return null;
  if (props.hub !== "online") return blocked.offline;
  if (!props.daemonReady) return blocked.starting;
  if (chat.pending?.kind === "approval") return blocked.permission;
  if (chat.pending?.kind === "clarify") return blocked.question;
  if (chat.activity === "transcribing") return blocked.transcribing;
  if (turnInFlight(chat)) return blocked.answering;
  if (uploading) return blocked.uploading;
  return null;
}

type Attachments = ReturnType<typeof useAttachmentDrafts>;

/**
 * Sends what is written, with the files ready. Nothing written is lost: the
 * box empties (`emptied`) only once the daemon accepted the message;
 * refused (`false`, or a failure) it stays, and the view says why.
 */
function sendKeeping(
  send: VoiceSend,
  written: string,
  attachments: Attachments,
  setSending: (sending: boolean) => void,
  emptied: () => void,
) {
  const files = attachments.ready;
  const text = written.trim();
  setSending(true);
  void Promise.resolve(
    send(
      files.length > 0 ? { command: "chat", text, attachments: files } : { command: "chat", text },
    ),
  )
    .then((accepted) => {
      if (accepted === false) return;
      emptied();
      attachments.clear();
    })
    .catch(() => undefined)
    .finally(() => setSending(false));
}

export function useComposer(props: ComposerProps, es: Copy) {
  const { attachmentLimits, chat, daemonReady, hub, paired, send } = props;
  const [draft, setDraft] = useState("");
  const box = useRef<HTMLTextAreaElement>(null);
  // «Responder a esto» quotes a fragment after what is already written.
  const insert = useCallback((text: string) => {
    setDraft((written) => (written.trim() === "" ? text : `${written.trimEnd()}\n\n${text}`));
    box.current?.focus();
  }, []);
  useComposerInsert(insert);
  const unavailable = !daemonReady || !paired || hub !== "online";
  const attachments = useAttachmentDrafts(attachmentLimits);
  // A message on its way: the box keeps it until the daemon has taken it.
  const [sending, setSending] = useState(false);
  const reason = sending
    ? es.assistant.blocked.sending
    : blockedReason(props, attachments.uploading, es);
  // The agent busy with this conversation: a new message or note waits.
  const busy = chat.pending !== null || chat.activity !== undefined;
  const voice = useVoiceNote({
    canStart: !unavailable && !busy,
    daemonListening: chat.voice.state === "listening",
    send,
  });
  // Text, or files without text; never while a file is still uploading.
  const canSend =
    !unavailable &&
    !busy &&
    !attachments.uploading &&
    !sending &&
    (draft.trim() !== "" || attachments.ready.length > 0);
  const submit = () => {
    if (!canSend) return;
    const written = draft;
    sendKeeping(send, written, attachments, setSending, () =>
      setDraft((now) => (now === written ? "" : now)),
    );
  };
  const keyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      submit();
    }
  };
  return {
    attachments,
    box,
    busy,
    canSend,
    draft,
    keyDown,
    reason,
    setDraft,
    submit,
    unavailable,
    voice,
  };
}

export type Controls = ReturnType<typeof useComposer>;
