import { Fragment } from "react";
import type { AgentWork } from "../../bridge/generated/AgentWork";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { VoiceNoteAudioSnapshot } from "../../bridge/generated/VoiceNoteAudioSnapshot";
import { Icon } from "../../design/Icon";
import { es } from "../../i18n/es";
import { AgentWorkView } from "./AgentWork";
import { AssistantAnswer } from "./AssistantAnswer";
import { AttachmentGallery } from "./AttachmentGallery";
import { MessageAttachments } from "./MessageAttachments";
import { retryText } from "./conversation-markdown";
import { UserText } from "./UserText";
import { VoiceNote, type VoiceNoteProps } from "./VoiceNote";
import styles from "./AssistantView.module.css";

interface Answering {
  /** A turn is in flight: «Reintentar» waits for it. */
  busy?: boolean;
  /** The answer being read aloud, by its message id. */
  speaking?: string | null;
}

function Message({
  answering,
  message,
  retry,
  send,
  voiceAudio,
}: {
  answering: Answering;
  message: ChatMessage;
  retry: string | null;
  send: VoiceNoteProps["send"];
  voiceAudio?: VoiceNoteAudioSnapshot;
}) {
  const assistant = message.role === "assistant";
  return (
    <article className={styles.message} data-role={message.role}>
      <div className={styles.bubble}>
        {assistant && <strong className={styles.messageLabel}>{es.assistant.orbit}</strong>}
        {message.voice ? (
          <VoiceNote
            audio={voiceAudio?.message_id === message.id ? voiceAudio : undefined}
            messageId={message.id}
            send={send}
            voice={message.voice}
          />
        ) : assistant ? (
          // Deimos answers in Markdown; the user's own text stays as they wrote it.
          <AssistantAnswer
            busy={answering.busy ?? false}
            message={message}
            retry={retry}
            send={send}
            speaking={answering.speaking ?? null}
          />
        ) : (
          <UserText text={message.text} />
        )}
        {message.attachments !== undefined && message.attachments.length > 0 && (
          <MessageAttachments attachments={message.attachments} />
        )}
        {message.streaming && <span aria-hidden="true" className={styles.cursor} />}
        {message.receipts.length > 0 && (
          <ul className={styles.receipts}>
            {message.receipts.map((receipt) => (
              <li key={`${receipt.kind}-${receipt.path}`}>
                <span className={styles.receiptCheck}>
                  <Icon name="check" size={10} />
                </span>
                <span>{receipt.label}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </article>
  );
}

export function MessageList({
  messages,
  send,
  voiceAudio,
  work,
  workInFlight = false,
  ...answering
}: {
  messages: ChatMessage[];
  send: VoiceNoteProps["send"];
  voiceAudio?: VoiceNoteAudioSnapshot;
  /** What the agent did in the last turn; it sits after that turn's question. */
  work?: AgentWork | null;
  workInFlight?: boolean;
} & Answering) {
  if (messages.length === 0) return <p className={styles.empty}>{es.chat.empty}</p>;
  return (
    <AttachmentGallery messages={messages}>
      {messages.map((message, index) => (
        <Fragment key={message.id}>
          <Message
            answering={answering}
            message={message}
            retry={message.role === "assistant" ? retryText(messages, index) : null}
            send={send}
            voiceAudio={voiceAudio}
          />
          {work != null && message.role === "user" && message.id === work.turn_id && (
            <AgentWorkView inFlight={workInFlight} work={work} />
          )}
        </Fragment>
      ))}
    </AttachmentGallery>
  );
}
