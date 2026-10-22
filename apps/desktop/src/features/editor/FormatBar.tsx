import type { EditorView } from "@codemirror/view";
import { Icon } from "../../design/Icon";
import { Tooltip } from "../../design/Tooltip";
import { es } from "../../i18n/es";
import { toggleWrap } from "./editing";
import type { EditorAssistant } from "./MarkdownEditor";
import styles from "./MarkdownEditor.module.css";

/** How far over the selected line the bar sits: its 38 points and a gap of 8 (prototype). */
const BAR_ABOVE = 46;

const MARKS = [
  { glyph: "B", tip: es.editor.bold, open: "**", close: "**", kind: "bold" },
  { glyph: "I", tip: es.editor.italic, open: "*", close: "*", kind: "italic" },
  { glyph: "</>", tip: es.editor.inlineCode, open: "`", close: "`", kind: "mono" },
  { glyph: "[[ ]]", tip: es.editor.linkTo, open: "[[", close: "]]", kind: "mono" },
  { glyph: "A", tip: es.editor.highlight, open: "==", close: "==", kind: "highlight" },
] as const;

/**
 * The bar over a selection: negrita, cursiva, código, enlazar a una nota,
 * resaltar and «Preguntar a {principal}». Each mark goes around the text
 * chosen, or comes off it when it is already there.
 */
export function FormatBar({
  assistant,
  place,
  selection,
  view,
}: {
  assistant?: EditorAssistant;
  place: { top: number; left: number };
  selection: { from: number; to: number };
  view: () => EditorView | null;
}) {
  const onWrap = (open: string, close: string) => {
    const editor = view();
    if (editor) toggleWrap(editor, open, close);
  };
  const onAsk = () => assistant?.ask(view()?.state.sliceDoc(selection.from, selection.to) ?? "");
  return (
    <div
      aria-label={es.editor.formatBar}
      className={styles.formatBar}
      role="toolbar"
      style={{ top: place.top - BAR_ABOVE, left: place.left }}
    >
      {MARKS.map((mark) => (
        <Tooltip key={mark.tip} text={mark.tip}>
          <button
            aria-label={mark.tip}
            className={styles.format}
            data-kind={mark.kind}
            onClick={() => onWrap(mark.open, mark.close)}
            // The selection stays: a press on the bar would otherwise clear it.
            onMouseDown={(event) => event.preventDefault()}
            type="button"
          >
            {mark.glyph}
          </button>
        </Tooltip>
      ))}
      {assistant !== undefined && (
        <>
          <span aria-hidden="true" className={styles.formatRule} />
          <button
            className={styles.ask}
            onClick={onAsk}
            onMouseDown={(event) => event.preventDefault()}
            type="button"
          >
            <Icon name="ask" size={14} />
            {es.editor.ask(assistant.name)}
          </button>
        </>
      )}
    </div>
  );
}
