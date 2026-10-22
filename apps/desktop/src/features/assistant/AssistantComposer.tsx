import { useState, type KeyboardEvent } from "react";
import type { ChatSnapshot } from "../../bridge/generated/ChatSnapshot";
import type { HubLinkConnection } from "../../bridge/generated/HubLinkConnection";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { TextLink } from "../../design/TextLink";
import { es } from "../../i18n/es";
import { mmss } from "../../time-format";
import { useVoiceNote, type VoiceSend } from "./useVoiceNote";
import styles from "./AssistantView.module.css";

interface ComposerProps {
  callActive: boolean;
  callPending: boolean;
  chat: ChatSnapshot;
  daemonReady: boolean;
  hub: HubLinkConnection;
  paired: boolean;
  /** Not known yet: the button waits, quietly. */
  callUnknown?: boolean;
  voiceUnknown?: boolean;
  statusUnknown?: boolean;
  onCallToggle: () => void;
  send: VoiceSend;
}

function useComposer({ chat, daemonReady, hub, paired, send, voiceUnknown }: ComposerProps) {
  const [draft, setDraft] = useState("");
  const disabled = !daemonReady || !paired || hub !== "online";
  const voice = useVoiceNote({
    canStart: !disabled && voiceUnknown !== true,
    daemonListening: chat.voice.state === "listening",
    send,
  });

  const submit = () => {
    const text = draft.trim();
    if (!text || disabled) return;
    send({ command: "chat", text });
    setDraft("");
  };
  const keyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      submit();
    }
  };
  return { disabled, draft, keyDown, setDraft, submit, voice };
}

type Controls = ReturnType<typeof useComposer>;

function MicrophoneButton({
  controls,
  voiceUnknown,
}: {
  controls: Controls;
  voiceUnknown?: boolean;
}) {
  const { voice } = controls;
  return (
    <Button
      aria-label={
        voice.recording
          ? es.assistant.voiceNoteRecording(mmss(voice.elapsedMs, false))
          : es.assistant.voiceNote
      }
      aria-pressed={voice.recording}
      className={voice.recording ? styles.recordingButton : undefined}
      // A note in progress can always end, whatever happened meanwhile.
      disabled={!voice.recording && (controls.disabled || voiceUnknown === true)}
      onClick={voice.keyboardClick}
      onPointerCancel={voice.pointerCancel}
      onPointerDown={voice.pointerDown}
      onPointerUp={voice.pointerUp}
      size={30}
      variant="icon"
    >
      <Icon name="mic" size={14} />
    </Button>
  );
}

// There is no «Compartir pantalla» button. What lets Deimos see the
// screen is the Screen Recording permission the user grants in macOS.
function ComposerActions({
  callActive,
  callPending,
  controls,
  onCallToggle,
  callUnknown,
  voiceUnknown,
}: {
  callActive: boolean;
  callPending: boolean;
  controls: Controls;
  onCallToggle: () => void;
  callUnknown?: boolean;
  voiceUnknown?: boolean;
}) {
  const recording = controls.voice.recording;
  return (
    <div className={styles.composerActions}>
      <Button
        aria-label={callActive ? es.design.hangUp : es.assistant.call}
        disabled={!callActive && (controls.disabled || callPending || callUnknown === true)}
        onClick={onCallToggle}
        size={30}
        variant="icon"
      >
        <Icon name="phone" size={14} />
      </Button>
      <MicrophoneButton controls={controls} voiceUnknown={voiceUnknown} />
      <Button
        aria-label={recording ? es.assistant.sendVoiceNote : es.chat.send}
        disabled={!recording && (controls.disabled || !controls.draft.trim())}
        onClick={recording ? controls.voice.finish : controls.submit}
        size={30}
        variant="accent"
      >
        <Icon name="send" size={14} />
      </Button>
    </div>
  );
}

/** What the composer says about the voice note: recording, or not sent. */
function VoiceLine({ chat, controls }: { chat: ChatSnapshot; controls: Controls }) {
  if (controls.voice.recording)
    return (
      <p className={styles.listening} role="status">
        <span aria-hidden="true" className={styles.recordingDot} />
        {es.assistant.recordingVoiceNote(mmss(controls.voice.elapsedMs, false))}
      </p>
    );
  const short = chat.voice.too_short;
  if (!short) return null;
  return (
    <p className={styles.voiceRefused} role="status">
      {es.assistant.voiceNoteTooShort(short.duration_ms, short.minimum_ms)}
    </p>
  );
}

export function AssistantComposer(props: ComposerProps) {
  const controls = useComposer(props);
  // Only what the user can fix has words here; offline and Orbit's own problems
  // are the status pill's, and an unknown says nothing.
  const unpaired = props.daemonReady && !props.paired && props.statusUnknown !== true;
  return (
    <footer className={styles.composerArea}>
      {unpaired && (
        <p className={styles.availability}>
          <TextLink href="#/settings">{es.assistant.pairHub}</TextLink>
        </p>
      )}
      <div className={styles.composer}>
        <textarea
          aria-label={es.chat.inputLabel}
          disabled={controls.disabled}
          onChange={(event) => controls.setDraft(event.target.value)}
          onKeyDown={controls.keyDown}
          placeholder={es.assistant.composerPlaceholder}
          rows={2}
          value={controls.draft}
        />
        <VoiceLine chat={props.chat} controls={controls} />
        <ComposerActions
          callActive={props.callActive}
          callPending={props.callPending}
          controls={controls}
          onCallToggle={props.onCallToggle}
          callUnknown={props.callUnknown}
          voiceUnknown={props.voiceUnknown}
        />
      </div>
    </footer>
  );
}
