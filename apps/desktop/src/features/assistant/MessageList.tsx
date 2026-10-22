import { Fragment, type ReactNode } from "react";
import type { AgentWork } from "../../bridge/generated/AgentWork";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { VoiceNoteAudioSnapshot } from "../../bridge/generated/VoiceNoteAudioSnapshot";
import { Icon } from "../../design/Icon";
import { useSpanish } from "../../i18n/AssistantCopy";
import { AgentWorkView, AnswerHead, type StepWait } from "./AgentWork";
import { AssistantAnswer } from "./AssistantAnswer";
import { AttachmentGallery } from "./AttachmentGallery";
import { CopyButton } from "./CopyButton";
import { MessageAttachments } from "./MessageAttachments";
import { retryText } from "./conversation-markdown";
import { VoiceNote, type VoiceNoteProps } from "./VoiceNote";
import styles from "./AssistantView.module.css";

interface Answering {
  /** A turn is in flight: «Volver a preguntar» waits for it. */
  busy?: boolean;
  /** The answer being read aloud, by its message id. */
  speaking?: string | null;
  /** «Escuchar» is offered: some voice can read this conversation's answers. */
  listenable?: boolean;
}

/** A message of the user: its files above, its words in a bubble, «Copiar» on hover. */
function UserMessage({
  message,
  send,
  voiceAudio,
}: {
  message: ChatMessage;
  send: VoiceNoteProps["send"];
  voiceAudio?: VoiceNoteAudioSnapshot;
}) {
  const es = useSpanish();
  return (
    <article aria-label={es.assistant.yourMessage} className={styles.user}>
      {message.attachments !== undefined && message.attachments.length > 0 && (
        <MessageAttachments attachments={message.attachments} />
      )}
      {message.voice ? (
        <VoiceNote
          audio={voiceAudio?.message_id === message.id ? voiceAudio : undefined}
          messageId={message.id}
          send={send}
          voice={message.voice}
        />
      ) : (
        message.text !== "" && (
          <>
            <p className={styles.bubble}>{message.text}</p>
            <div className={styles.userActions}>
              <CopyButton label={es.chatActions.copyMessage} text={() => message.text} />
            </div>
          </>
        )
      )}
    </article>
  );
}

/** An answer, without a bubble: the agent and its work above, then its Markdown. */
function Answer({
  answering,
  head,
  message,
  retry,
  send,
}: {
  answering: Answering;
  head: ReactNode;
  message: ChatMessage;
  retry: string | null;
  send: VoiceNoteProps["send"];
}) {
  const es = useSpanish();
  return (
    <article aria-label={es.assistant.answerLabel} className={styles.answer}>
      {head}
      <AssistantAnswer
        busy={answering.busy ?? false}
        listenable={answering.listenable ?? true}
        message={message}
        retry={retry}
        send={send}
        speaking={answering.speaking ?? null}
      />
      {message.streaming && <span aria-hidden="true" className={styles.cursor} />}
      {message.receipts.length > 0 && (
        <ul className={styles.receipts}>
          {message.receipts.map((receipt) => (
            <li key={`${receipt.kind}-${receipt.path}`}>
              <span className={styles.receiptCheck}>
                <Icon name="check" size={14} />
              </span>
              <span>{receipt.label}</span>
            </li>
          ))}
        </ul>
      )}
    </article>
  );
}

/**
 * The conversation: the user's messages on the right, the agent's answers
 * at reading width with what it did on their first line. A turn still
 * without an answer shows its work alone, and `children` (what the agent
 * waits for, a failure) after it.
 */
export function MessageList({
  children,
  messages,
  send,
  voiceAudio,
  work,
  workInFlight = false,
  wait = null,
  ...answering
}: {
  children?: ReactNode;
  messages: ChatMessage[];
  send: VoiceNoteProps["send"];
  voiceAudio?: VoiceNoteAudioSnapshot;
  /** What the agent did in the last turn; it heads that turn's answer. */
  work?: AgentWork | null;
  workInFlight?: boolean;
  /** What the running step of the turn in flight waits for. */
  wait?: StepWait;
} & Answering) {
  const es = useSpanish();
  const asked = work == null ? -1 : messages.findIndex((message) => message.id === work.turn_id);
  const answered = asked >= 0 && messages[asked + 1]?.role === "assistant";
  // An ended turn's work is folded even while the next one runs.
  const working = workInFlight && work?.ended_at_ms == null;
  const workView =
    work == null ? null : <AgentWorkView inFlight={working} wait={wait} work={work} />;
  return (
    <AttachmentGallery messages={messages}>
      {messages.map((message, index) => (
        <Fragment key={message.id}>
          {message.role === "assistant" ? (
            <Answer
              answering={answering}
              head={answered && index === asked + 1 ? workView : <AnswerHead />}
              message={message}
              retry={retryText(messages, index)}
              send={send}
            />
          ) : (
            <UserMessage message={message} send={send} voiceAudio={voiceAudio} />
          )}
          {index === asked && !answered && (
            <article aria-label={es.assistant.workingLabel} className={styles.answer}>
              {workView}
            </article>
          )}
        </Fragment>
      ))}
      {children}
    </AttachmentGallery>
  );
}
