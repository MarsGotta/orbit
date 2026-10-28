import { useId, type ReactNode } from "react";
import type { ChatSnapshot } from "../../bridge/generated/ChatSnapshot";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { useCopy } from "../../i18n/CopyProvider";
import { mmss } from "../../time-format";
import { AttachButtons, AttachmentChips } from "./AttachmentChips";
import { useComposer, type ComposerProps, type Controls } from "./use-composer";
import styles from "./AssistantView.module.css";

/**
 * The microphone: a click starts a note and another sends it; held, it
 * talks while held. While the note records it stays, as the red dot that
 * begins the recording strip, so the release of a held press still reaches it.
 */
function MicrophoneButton({ controls }: { controls: Controls }) {
  const es = useCopy();
  const { voice } = controls;
  return (
    <Button
      aria-label={
        voice.recording
          ? es.assistant.voiceNoteRecording(mmss(voice.elapsedMs, false))
          : es.assistant.voiceNoteButton
      }
      aria-pressed={voice.recording}
      className={voice.recording ? styles.micRecording : styles.mic}
      // A note in progress can always end, whatever happened meanwhile.
      disabled={!voice.recording && (controls.unavailable || controls.busy)}
      onClick={voice.keyboardClick}
      onPointerCancel={voice.pointerCancel}
      onPointerDown={voice.pointerDown}
      onPointerUp={voice.pointerUp}
      size={32}
      variant="icon"
    >
      {voice.recording ? (
        <span aria-hidden="true" className={styles.recordingDot} />
      ) : (
        <Icon name="mic" size={16} />
      )}
    </Button>
  );
}

/** A note too short to send says its length and the minimum, here where it was recorded. */
function TooShort({ chat }: { chat: ChatSnapshot }) {
  const es = useCopy();
  const short = chat.voice.too_short;
  if (!short) return null;
  return (
    <p className={styles.voiceRefused} role="status">
      {es.assistant.voiceNoteTooShort(short.duration_ms, short.minimum_ms)}
    </p>
  );
}

/** «Adjuntar», «Adjuntar ventana», the microphone and «Enviar»; while a note records, its strip. */
function Toolbar({ controls, reasonId }: { controls: Controls; reasonId: string }) {
  const es = useCopy();
  const recording = controls.voice.recording;
  return (
    <div className={styles.toolbar}>
      {!recording && (
        <AttachButtons
          disabled={controls.unavailable}
          onAttach={controls.attachments.choose}
          onAttachWindow={controls.attachments.addWindow}
          supported={controls.attachments.supported}
        />
      )}
      <span className={styles.spacer} />
      <MicrophoneButton controls={controls} />
      {recording ? (
        <>
          <p className={styles.recordingLine} role="status">
            <span>{es.assistant.recordingNote}</span>
            <time>{mmss(controls.voice.elapsedMs, false)}</time>
            <span className={styles.escHint}>{es.assistant.escCancels}</span>
          </p>
          <Button onClick={controls.voice.cancel} size={32} variant="ghost">
            {es.assistant.cancel}
          </Button>
          <Button onClick={controls.voice.finish} size={32} variant="accent">
            {es.assistant.sendVoiceNote}
          </Button>
        </>
      ) : (
        <Button
          aria-describedby={controls.reason !== null ? reasonId : undefined}
          aria-label={es.chat.send}
          className={styles.send}
          disabled={!controls.canSend}
          onClick={controls.submit}
          size={32}
          variant="accent"
        >
          <Icon name="send" size={16} />
        </Button>
      )}
    </div>
  );
}

/**
 * The composer, fixed under the thread: the files of the next message, the
 * text, «Adjuntar», «Adjuntar ventana», the microphone and «Enviar». While
 * the agent answers or waits, sending waits too and says why; the text stays.
 * While a voice note records, the box becomes its strip: the time, «Cancelar»
 * and «Enviar nota de voz».
 */
export function AssistantComposer(props: ComposerProps & { talkingTo?: ReactNode }) {
  const es = useCopy();
  const controls = useComposer(props, es);
  const reasonId = useId();
  const recording = controls.voice.recording;
  return (
    <footer className={styles.composerArea}>
      <div className={styles.composerColumn}>
        {controls.reason !== null && (
          <p aria-live="polite" className={styles.blocked} id={reasonId}>
            {controls.reason}
          </p>
        )}
        <TooShort chat={props.chat} />
        {controls.recovered && (
          <p className={styles.blocked} role="status">
            {es.assistant.draftRecovered}
          </p>
        )}
        <div className={styles.composer} data-recording={recording || undefined}>
          <div className={styles.writing} hidden={recording}>
            {props.talkingTo}
            <AttachmentChips
              drafts={controls.attachments.drafts}
              onRemove={controls.attachments.remove}
            />
            <textarea
              aria-label={es.assistant.composerLabel}
              onChange={(event) => controls.setDraft(event.target.value)}
              onKeyDown={controls.keyDown}
              onPaste={controls.attachments.paste}
              placeholder={es.assistant.composerPlaceholder}
              ref={controls.box}
              rows={2}
              value={controls.draft}
            />
          </div>
          <Toolbar controls={controls} reasonId={reasonId} />
        </div>
      </div>
    </footer>
  );
}
