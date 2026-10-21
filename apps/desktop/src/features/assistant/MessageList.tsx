import { useEffect, useRef, useState } from "react";
import type { AppCommand } from "../../bridge/generated/AppCommand";
import type { ChatMessage } from "../../bridge/generated/ChatMessage";
import type { VoiceNote as VoiceNoteSnapshot } from "../../bridge/generated/VoiceNote";
import type { VoiceNoteAudioSnapshot } from "../../bridge/generated/VoiceNoteAudioSnapshot";
import { Icon } from "../../design/Icon";
import { mmss } from "../../time-format";
import { es } from "../../i18n/es";
import { MarkdownView } from "../notes/MarkdownView";
import styles from "./AssistantView.module.css";

interface VoiceNoteProps {
  messageId: string;
  send: (command: AppCommand) => void;
  voice: VoiceNoteSnapshot;
  /** The audio Mars last asked to hear, when it is this note's. */
  audio?: VoiceNoteAudioSnapshot;
}

function play(media: HTMLMediaElement | null): void {
  if (media) void media.play().catch(() => undefined);
}

function voiceTranscript(voice: VoiceNoteSnapshot): string {
  return (
    voice.transcript ??
    (voice.failure ? es.assistant.voiceFailure(voice.failure) : es.assistant.transcribing)
  );
}

function VoiceNote({ audio, messageId, send, voice }: VoiceNoteProps) {
  const audioRef = useRef<HTMLAudioElement>(null);
  const [requested, setRequested] = useState(false);
  const [source, setSource] = useState<string | null>(null);
  const [progress, setProgress] = useState(0);
  const available = voice.audio_available ?? false;

  useEffect(() => {
    if (audio == null || source != null) return;
    setSource(`data:audio/${audio.format};base64,${audio.bytes_base64}`);
  }, [audio, source]);

  useEffect(() => {
    if (source != null) play(audioRef.current);
  }, [source]);

  const start = () => {
    if (!available) return;
    if (source != null) {
      play(audioRef.current);
      return;
    }
    if (requested) return;
    setRequested(true);
    send({ command: "voice_note_audio", message_id: messageId });
  };
  const seconds = Math.max(0, voice.duration_ms / 1_000);
  return (
    <div>
      <div className={styles.voiceNote}>
        <button
          aria-label={available ? es.assistant.playVoiceNote : es.assistant.audioUnavailable}
          disabled={!available}
          onClick={start}
          type="button"
        >
          <Icon name="play" size={10} />
        </button>
        <span aria-hidden="true" className={styles.waveform}>
          {Array.from({ length: 8 }, (_, index) => (
            <i key={index} />
          ))}
        </span>
        <time>{mmss(voice.duration_ms, false)}</time>
      </div>
      <progress
        aria-label={es.assistant.voiceProgress}
        className={styles.voiceProgress}
        max={seconds}
        value={progress}
      />
      <audio
        data-testid={`voice-audio-${messageId}`}
        hidden
        onEnded={() => setProgress(0)}
        onTimeUpdate={(event) => setProgress(event.currentTarget.currentTime)}
        ref={audioRef}
        src={source ?? undefined}
      />
      <p className={styles.voiceTranscript}>{voiceTranscript(voice)}</p>
    </div>
  );
}

function Message({
  message,
  send,
  voiceAudio,
}: {
  message: ChatMessage;
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
          // SIM-30: Deimos answers in Markdown; Mars's own text stays as she wrote it.
          <MarkdownView source={message.text} />
        ) : (
          <p>{message.text}</p>
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
}: {
  messages: ChatMessage[];
  send: VoiceNoteProps["send"];
  voiceAudio?: VoiceNoteAudioSnapshot;
}) {
  if (messages.length === 0) return <p className={styles.empty}>{es.chat.empty}</p>;
  return messages.map((message) => (
    <Message key={message.id} message={message} send={send} voiceAudio={voiceAudio} />
  ));
}
