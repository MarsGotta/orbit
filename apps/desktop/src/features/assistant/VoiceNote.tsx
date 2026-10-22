import { useEffect, useRef, useState } from "react";
import type { AppCommand } from "../../bridge/generated/AppCommand";
import type { VoiceNote as VoiceNoteSnapshot } from "../../bridge/generated/VoiceNote";
import type { VoiceNoteAudioSnapshot } from "../../bridge/generated/VoiceNoteAudioSnapshot";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { mmss } from "../../time-format";
import { es } from "../../i18n/es";
import styles from "./AssistantView.module.css";

export interface VoiceNoteProps {
  messageId: string;
  send: (command: AppCommand) => void;
  voice: VoiceNoteSnapshot;
  /** The audio the user last asked to hear, when it is this note's. */
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

/**
 * The note's transcript, or why there is none. A failed transcription can be
 * asked for again from the audio this Mac keeps; without that audio there is
 * nothing to send again.
 */
function VoiceTranscript({ messageId, send, voice }: Omit<VoiceNoteProps, "audio">) {
  const retry = voice.failure != null && voice.transcript == null && voice.audio_available;
  return (
    <>
      <p className={styles.voiceTranscript}>{voiceTranscript(voice)}</p>
      {retry === true && (
        <Button
          onClick={() => send({ command: "voice_retry", message_id: messageId })}
          size={26}
          variant="ghost"
        >
          {es.assistant.retryTranscription}
        </Button>
      )}
    </>
  );
}

export function VoiceNote({ audio, messageId, send, voice }: VoiceNoteProps) {
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
      <VoiceTranscript messageId={messageId} send={send} voice={voice} />
    </div>
  );
}
