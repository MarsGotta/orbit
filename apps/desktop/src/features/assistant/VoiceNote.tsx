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
  send: (command: AppCommand) => unknown;
  voice: VoiceNoteSnapshot;
  /** The audio the user last asked to hear, when it is this note's. */
  audio?: VoiceNoteAudioSnapshot;
}

// The drawn wave of a note, as the design draws it: not its audio's levels.
const WAVE = [6, 12, 16, 9, 14, 7, 12, 5, 10, 6];

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
      <p className={styles.voiceTranscript} data-failed={voice.failure != null || undefined}>
        {voiceTranscript(voice)}
      </p>
      {retry === true && (
        <Button
          onClick={() => send({ command: "voice_retry", message_id: messageId })}
          size={28}
          variant="ghost"
        >
          {es.assistant.retryTranscription}
        </Button>
      )}
    </>
  );
}

/**
 * Asks the app for a note's audio, once at a time; once answered (refused or
 * not) it can be asked again: the audio, when it came, is already on its way.
 */
function useAudioRequest(messageId: string, send: VoiceNoteProps["send"]) {
  const [requested, setRequested] = useState(false);
  return () => {
    if (requested) return;
    setRequested(true);
    void Promise.resolve(send({ command: "voice_note_audio", message_id: messageId })).finally(() =>
      setRequested(false),
    );
  };
}

export function VoiceNote({ audio, messageId, send, voice }: VoiceNoteProps) {
  const audioRef = useRef<HTMLAudioElement>(null);
  const request = useAudioRequest(messageId, send);
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
    request();
  };
  const seconds = Math.max(0, voice.duration_ms / 1_000);
  return (
    <div className={styles.voice}>
      <div className={styles.voiceNote}>
        <button
          aria-label={available ? es.assistant.playVoiceNote : es.assistant.audioUnavailable}
          disabled={!available}
          onClick={start}
          type="button"
        >
          <Icon name="play" size={14} />
        </button>
        <span aria-hidden="true" className={styles.waveform}>
          {WAVE.map((height, index) => (
            <i key={index} style={{ height }} />
          ))}
        </span>
        <time>{mmss(voice.duration_ms, false)}</time>
        <progress
          aria-label={es.assistant.voiceProgress}
          className={styles.voiceProgress}
          max={seconds}
          value={progress}
        />
      </div>
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
