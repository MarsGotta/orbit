import { useEffect, useId, useState, type KeyboardEvent, type ReactNode } from "react";
import { Icon } from "../design/Icon";
import { useModalFocus } from "../design/use-modal-focus";
import { useSpanish } from "../i18n/AssistantCopy";
import { matchCommands, paletteGroups, type PaletteCommand } from "./palette-commands";
import { AskRow, CommandRow } from "./PaletteRows";
import styles from "./Palette.module.css";

export interface PaletteProps {
  commands: PaletteCommand[];
  /** Sends what was written to the agent, when the person picks asking it. */
  onAsk: (text: string) => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}

/** ⌘⇧␣, only while Orbit's window has the focus. */
function isPaletteShortcut(event: globalThis.KeyboardEvent): boolean {
  return event.code === "Space" && event.metaKey && event.shiftKey;
}

/** One row of the list: either a command or asking the agent. */
type Row = { kind: "command"; command: PaletteCommand } | { kind: "ask"; text: string };

/**
 * The rows in the order ↑ ↓ walk them. What matches no command makes asking
 * the agent the first row; with matches, asking stays reachable after them.
 */
function rowsFor(commands: PaletteCommand[], query: string): Row[] {
  const matches = matchCommands(commands, query).map((command): Row => ({
    kind: "command",
    command,
  }));
  const text = query.trim();
  if (text === "") return matches;
  const ask: Row = { kind: "ask", text };
  return matches.length === 0 ? [ask] : [...matches, ask];
}

/** «Buscar y órdenes» (⌘⇧␣): the window's actions, views and «Parar todo». */
export function Palette(props: PaletteProps) {
  const { onOpenChange, open } = props;
  useEffect(() => {
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (!isPaletteShortcut(event)) return;
      event.preventDefault();
      onOpenChange(!open);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onOpenChange, open]);
  // Mounted only while open, so each opening starts empty and at the top.
  return open ? <OpenPalette {...props} /> : null;
}

/** What is written, the rows it leaves and the one selected, with ↑ ↓ ↵ and Esc. */
function usePaletteState({ commands, onAsk, onOpenChange }: PaletteProps) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const rows = rowsFor(commands, query);
  const current = Math.min(selected, Math.max(rows.length - 1, 0));
  const choose = (index: number) => {
    const row = rows[index];
    if (!row) return;
    onOpenChange(false);
    if (row.kind === "ask") onAsk(row.text);
    else row.command.run();
  };
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const step = event.key === "ArrowDown" ? 1 : -1;
      setSelected(Math.min(Math.max(current + step, 0), Math.max(rows.length - 1, 0)));
    } else if (event.key === "Enter") {
      event.preventDefault();
      choose(current);
    } else if (event.key === "Escape") {
      event.preventDefault();
      onOpenChange(false);
    }
  };
  const write = (text: string) => {
    setQuery(text);
    setSelected(0);
  };
  return { query, write, rows, current, setSelected, choose, onKeyDown };
}

function OpenPalette(props: PaletteProps) {
  const copy = useSpanish().palette;
  const state = usePaletteState(props);
  const panel = useModalFocus(true);
  const ids = useId();
  const optionId = (index: number) => `${ids}-option-${index}`;
  const { onOpenChange } = props;
  return (
    <div
      className={styles.backdrop}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onOpenChange(false);
      }}
      role="presentation"
    >
      <section
        ref={panel}
        aria-label={copy.dialog}
        aria-modal="true"
        className={styles.palette}
        role="dialog"
      >
        <div className={styles.search}>
          <Icon name="search" size={18} />
          <input
            aria-activedescendant={state.rows.length > 0 ? optionId(state.current) : undefined}
            aria-autocomplete="list"
            aria-controls={`${ids}-list`}
            aria-expanded="true"
            aria-label={copy.inputLabel}
            className={styles.input}
            onChange={(event) => state.write(event.currentTarget.value)}
            onKeyDown={state.onKeyDown}
            placeholder={copy.placeholder}
            role="combobox"
            value={state.query}
          />
          <button
            aria-label={copy.close}
            className={styles.escape}
            onClick={() => onOpenChange(false)}
            tabIndex={-1}
            type="button"
          >
            {copy.escape}
          </button>
        </div>
        <PaletteList listId={`${ids}-list`} optionId={optionId} state={state} />
        <div className={styles.footer}>
          <span>{copy.footer.move}</span>
          <span>{copy.footer.open}</span>
          <span>{copy.footer.close}</span>
        </div>
      </section>
    </div>
  );
}

/** The rows by group; asking the agent first when nothing else matches. */
function PaletteList(props: {
  listId: string;
  optionId: (index: number) => string;
  state: ReturnType<typeof usePaletteState>;
}) {
  const copy = useSpanish().palette;
  const { rows, current, choose, setSelected } = props.state;
  const option = (row: Row, index: number): ReactNode => (
    <button
      key={row.kind === "ask" ? "ask" : row.command.id}
      aria-selected={index === current}
      className={styles.option}
      id={props.optionId(index)}
      onClick={() => choose(index)}
      onMouseDown={(event) => event.preventDefault()}
      onMouseEnter={() => setSelected(index)}
      role="option"
      tabIndex={-1}
      type="button"
    >
      {row.kind === "ask" ? (
        <AskRow label={copy.ask} text={copy.quoted(row.text)} />
      ) : (
        <CommandRow command={row.command} />
      )}
    </button>
  );
  const indexed = rows.map((row, index) => ({ row, index }));
  const ask = indexed.find(({ row }) => row.kind === "ask");
  const noMatch = ask !== undefined && rows.length === 1;
  return (
    <div className={styles.scroll}>
      <div aria-label={copy.dialog} id={props.listId} role="listbox">
        {ask && noMatch && option(ask.row, ask.index)}
        {paletteGroups.map((group) => {
          const members = indexed.filter(
            ({ row }) => row.kind === "command" && row.command.group === group,
          );
          if (members.length === 0) return null;
          return (
            <div aria-label={copy.groups[group]} key={group} role="group">
              <div aria-hidden="true" className={styles.group}>
                {copy.groups[group]}
              </div>
              {members.map(({ row, index }) => option(row, index))}
            </div>
          );
        })}
        {ask && !noMatch && option(ask.row, ask.index)}
      </div>
      {noMatch && <p className={styles.noMatch}>{copy.noMatch}</p>}
    </div>
  );
}
