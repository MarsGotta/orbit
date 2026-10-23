import { useEffect, useRef, useState } from "react";
import type { AppCommand } from "../../bridge/generated/AppCommand";
import type { VoiceNote as VoiceNoteSnapshot } from "../../bridge/generated/VoiceNote";
import type { VoiceNoteAudioSnapshot } from "../../bridge/generated/VoiceNoteAudioSnapshot";
import { Button } from "../../design/Button";
import { ConfirmCard } from "../../design/ConfirmCard";
import { Icon } from "../../design/Icon";
import { mmss } from "../../time-format";
import { es } from "../../i18n/es";
import { providerRefusedAudio } from "../../i18n/es/voice-failures";
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
 * The note's transcript, or why there is none, in a calm line whose codes are
 * its detail (`title`). A failed transcription can be asked for again from the
 * audio this Mac keeps, unless the provider refused that audio: the same
 * audio would be refused again. A failed note can always be discarded, after
 * a question in place.
 */
function VoiceTranscript({ messageId, send, voice }: Omit<VoiceNoteProps, "audio">) {
  const [anchor, setAnchor] = useState<HTMLButtonElement | null>(null);
  const [asking, setAsking] = useState(false);
  const failure = voice.transcript == null ? voice.failure : null;
  const retry = failure != null && voice.audio_available === true && !providerRefusedAudio(failure);
  return (
    <>
      <p
        className={styles.voiceTranscript}
        data-failed={failure != null || undefined}
        title={failure != null ? es.assistant.voiceFailureDetail(failure) : undefined}
      >
        {voiceTranscript(voice)}
      </p>
      {failure != null && (
        <span className={styles.voiceActions}>
          {retry && (
            <Button
              onClick={() => send({ command: "voice_retry", message_id: messageId })}
              size={28}
              variant="ghost"
            >
              {es.assistant.retryTranscription}
            </Button>
          )}
          <Button
            aria-expanded={asking}
            onClick={() => setAsking(!asking)}
            ref={setAnchor}
            size={28}
            variant="ghost"
          >
            {es.assistant.discardVoiceNote}
          </Button>
        </span>
      )}
      {asking && (
        <ConfirmCard
          anchor={anchor}
          confirmLabel={es.assistant.discardVoiceNote}
          onClose={() => setAsking(false)}
          onConfirm={() => {
            setAsking(false);
            send({ command: "voice_discard", message_id: messageId });
          }}
          title={es.assistant.discardVoiceNoteTitle}
        >
          {es.assistant.discardVoiceNoteText}
        </ConfirmCard>
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
