import { useRef, useState, type KeyboardEvent } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { ChatSnapshot } from "../../bridge/generated/ChatSnapshot";
import type { HubLinkConnection } from "../../bridge/generated/HubLinkConnection";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { TextLink } from "../../design/TextLink";
import { es } from "../../i18n/es";
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
  send: (command: AppCommand) => void;
}

function useComposer({ chat, daemonReady, hub, paired, send, voiceUnknown }: ComposerProps) {
  const [draft, setDraft] = useState("");
  const dictating = useRef(false);
  const disabled = !daemonReady || !paired || hub !== "online";
  const listening = chat.voice.state === "listening" || dictating.current;

  const submit = () => {
    const text = draft.trim();
    if (!text || disabled) return;
    send({ command: "chat", text });
    setDraft("");
  };
  const startDictation = () => {
    if (disabled || voiceUnknown === true || dictating.current) return;
    dictating.current = true;
    send({ command: "voice_start" });
  };
  const endDictation = () => {
    if (!dictating.current) return;
    dictating.current = false;
    send({ command: "voice_end" });
  };
  const keyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      submit();
    }
  };
  return {
    disabled,
    draft,
    endDictation,
    keyDown,
    listening,
    setDraft,
    startDictation,
    submit,
  };
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
  controls: ReturnType<typeof useComposer>;
  onCallToggle: () => void;
  callUnknown?: boolean;
  voiceUnknown?: boolean;
}) {
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
      <Button
        aria-label={es.assistant.voiceNote}
        disabled={controls.disabled || voiceUnknown === true}
        onPointerCancel={controls.endDictation}
        onPointerDown={controls.startDictation}
        onPointerUp={controls.endDictation}
        size={30}
        variant="icon"
      >
        <Icon name="mic" size={14} />
      </Button>
      <Button
        aria-label={es.chat.send}
        disabled={controls.disabled || !controls.draft.trim()}
        onClick={controls.submit}
        size={30}
        variant="accent"
      >
        <Icon name="send" size={14} />
      </Button>
    </div>
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
        {controls.listening && (
          <div className={styles.listening}>
            <span>{es.assistant.listening}</span>
            <meter
              aria-label={es.assistant.microphoneLevel}
              max={100}
              min={0}
              value={props.chat.voice.level}
            />
          </div>
        )}
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
