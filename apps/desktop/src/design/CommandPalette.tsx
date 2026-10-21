import { type ReactNode, Fragment, useEffect, useState } from "react";
import { es } from "../i18n/es";
import { Icon, type IconName } from "./Icon";
import { Kbd } from "./Kbd";
import { useModalFocus } from "./use-modal-focus";
import styles from "./CommandPalette.module.css";

export interface CommandItem {
  icon: IconName;
  id: string;
  label: string;
  onSelect: () => void;
  subtitle: string;
  leading?: ReactNode;
  group?: string;
  checked?: boolean;
}

export interface CommandPaletteProps {
  commands: CommandItem[];
  onOpenChange?: (open: boolean) => void;
  open?: boolean;
  title?: string;
  inputLabel?: string;
  query?: string;
  onQueryChange?: (query: string) => void;
}

function OptionContent({ command, selected }: { command: CommandItem; selected: boolean }) {
  return (
    <>
      {command.leading ?? (
        <span className={styles.optionIcon}>
          <Icon name={command.icon} size={14} />
        </span>
      )}
      <span className={styles.optionCopy}>
        <strong>{command.label}</strong>
        <small>{command.subtitle}</small>
      </span>
      {/* ↵ belongs to the listbox, not to a command. It is painted
          only beside the selected option, and pressing it does what that
          option says; no other shortcut is announced here. */}
      {command.checked === true && <Icon name="check" size={14} />}
      {selected && <Kbd>↵</Kbd>}
    </>
  );
}

interface PaletteOptionsProps {
  commands: CommandItem[];
  execute: (index: number) => void;
  selected: number;
  setSelected: (index: number) => void;
}

function PaletteOptions({ commands, execute, selected, setSelected }: PaletteOptionsProps) {
  return commands.map((command, index) => {
    const interaction = {
      "aria-label": command.label,
      "aria-selected": selected === index,
      className: styles.option,
      onClick: () => execute(index),
      onMouseEnter: () => setSelected(index),
      role: "option",
    };
    return (
      <Fragment key={command.id}>
        {command.group && command.group !== commands[index - 1]?.group && (
          <div className={styles.group}>{command.group}</div>
        )}
        <button {...interaction} type="button">
          <OptionContent command={command} selected={selected === index} />
        </button>
      </Fragment>
    );
  });
}

function usePaletteControls({ commands, onOpenChange, open, query }: CommandPaletteProps) {
  const [internalOpen, setInternalOpen] = useState(false);
  const [selected, setSelected] = useState(0);
  const visible = open ?? internalOpen;
  useEffect(() => setSelected(0), [query]);
  const setOpen = (next: boolean) => {
    if (open === undefined) setInternalOpen(next);
    onOpenChange?.(next);
    if (next) setSelected(0);
  };
  const execute = (index: number) => {
    const command = commands[index];
    if (!command) return;
    command.onSelect();
    setOpen(false);
  };
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.code === "Space" && event.metaKey && event.shiftKey) {
        event.preventDefault();
        setOpen(!visible);
      } else if (visible && event.key === "Escape") setOpen(false);
      else if (visible && event.key === "ArrowDown") {
        event.preventDefault();
        setSelected((current) => (current + 1) % Math.max(commands.length, 1));
      } else if (visible && event.key === "ArrowUp") {
        event.preventDefault();
        setSelected((current) => (current - 1 + commands.length) % Math.max(commands.length, 1));
      } else if (visible && event.key === "Enter") {
        event.preventDefault();
        execute(selected);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [commands, selected, visible]);
  return { execute, selected, setOpen, setSelected, visible };
}

export function CommandPalette(props: CommandPaletteProps) {
  const { execute, selected, setOpen, setSelected, visible } = usePaletteControls(props);
  const panel = useModalFocus(visible);
  if (!visible) return null;
  return (
    <div
      className={styles.backdrop}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) setOpen(false);
      }}
      role="presentation"
    >
      <section
        ref={panel}
        aria-modal="true"
        aria-label={props.title ?? es.design.askOrbit}
        className={styles.palette}
        role="dialog"
      >
        <div className={styles.search}>
          <span aria-hidden="true" className={styles.orbitMark} />
          <input
            aria-label={props.inputLabel ?? es.design.commandInputLabel}
            placeholder={props.inputLabel ?? es.design.commandPlaceholder}
            value={props.query}
            onChange={(event) => props.onQueryChange?.(event.currentTarget.value)}
          />
          <span className={styles.escape}>{es.design.escape}</span>
        </div>
        <div className={styles.options} role="listbox">
          <PaletteOptions
            commands={props.commands}
            execute={execute}
            selected={selected}
            setSelected={setSelected}
          />
        </div>
        {/* There was a footer here promising ⇥ (change session),
            ⌥ (dictate) and «↵ llamar». None of them did anything while the
            palette was open, so it is gone; the only advertised shortcuts
            are the ones this palette answers to. */}
      </section>
    </div>
  );
}
