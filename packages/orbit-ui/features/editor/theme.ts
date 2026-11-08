import { EditorView } from "@codemirror/view";

/**
 * How the editor's text looks (guide 6, «El editor»): its classes are the
 * editor's own, so they live here with it rather than in a stylesheet.
 */
export const editorTheme = EditorView.theme({
  "&": {
    color: "var(--o-ink)",
    backgroundColor: "transparent",
    fontSize: "15px",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": {
    fontFamily: "var(--o-font-sans)",
    lineHeight: "1.7",
    overflow: "visible",
  },
  ".cm-content": {
    padding: "0",
    caretColor: "var(--o-accText)",
    minHeight: "1.7em",
  },
  ".cm-line": { padding: "0 6px", margin: "0 -6px", borderRadius: "6px" },
  ".cm-content ::selection, .cm-line::selection": { backgroundColor: "var(--o-sel)" },
  ".cm-cursor": { borderLeftColor: "var(--o-accText)", borderLeftWidth: "1.5px" },
  ".cm-placeholder": { color: "var(--o-ink3)" },

  ".cm-md-active": { backgroundColor: "var(--o-hover)" },
  ".cm-md-mark": { color: "var(--o-ink3)", fontWeight: "400", fontStyle: "normal" },

  ".cm-md-h1": {
    fontSize: "24px",
    fontWeight: "600",
    letterSpacing: "-0.02em",
    lineHeight: "1.35",
    paddingTop: "14px",
  },
  ".cm-md-h2": {
    fontSize: "20px",
    fontWeight: "600",
    letterSpacing: "-0.01em",
    lineHeight: "1.4",
    paddingTop: "12px",
  },
  ".cm-md-h3": { fontSize: "16px", fontWeight: "600", lineHeight: "1.5", paddingTop: "10px" },
  ".cm-line:first-child.cm-md-h1, .cm-line:first-child.cm-md-h2, .cm-line:first-child.cm-md-h3": {
    paddingTop: "0",
  },

  ".cm-md-strong": { fontWeight: "600" },
  // The app ships Inter without its italic and turns synthesis off; here the
  // italic is drawn slanted, so «*cursiva*» reads as such.
  ".cm-md-em": { fontStyle: "italic", fontSynthesis: "style" },
  ".cm-md-strike": { textDecoration: "line-through" },
  ".cm-md-code": {
    fontFamily: "var(--o-font-mono)",
    fontSize: "0.88em",
    padding: "1px 5px",
    borderRadius: "5px",
    backgroundColor: "var(--o-raised)",
  },
  ".cm-md-highlight": {
    backgroundColor: "var(--o-warnSoft)",
    borderRadius: "3px",
    padding: "0 2px",
  },
  ".cm-md-link": {
    color: "var(--o-accText)",
    textDecoration: "underline",
    textDecorationColor: "var(--o-line2)",
    textUnderlineOffset: "3px",
  },
  ".cm-md-mention": {
    color: "var(--o-accText)",
    backgroundColor: "var(--o-accSoft)",
    borderRadius: "5px",
    padding: "0 4px",
  },

  ".cm-md-item, .cm-md-task": { paddingLeft: "31px", textIndent: "-25px" },
  ".cm-md-bullet, .cm-md-number": {
    display: "inline-block",
    width: "17px",
    marginRight: "8px",
    color: "var(--o-ink3)",
    textIndent: "0",
    fontVariantNumeric: "tabular-nums",
  },
  ".cm-md-bullet": { textAlign: "center" },
  ".cm-md-number": { textAlign: "right" },
  ".cm-md-taskbox": {
    display: "inline-block",
    boxSizing: "border-box",
    width: "16px",
    height: "16px",
    marginRight: "9px",
    verticalAlign: "-2px",
    border: "1.5px solid var(--o-ink3)",
    borderRadius: "5px",
    cursor: "pointer",
    textIndent: "0",
  },
  ".cm-md-taskbox[data-checked=true]": {
    borderColor: "var(--o-acc)",
    backgroundColor: "var(--o-acc)",
    backgroundImage:
      "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='%23fff' stroke-width='3.2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M5 12.5l4.5 4.5L19 7.5'/%3E%3C/svg%3E\")",
    backgroundSize: "10px",
    backgroundPosition: "center",
    backgroundRepeat: "no-repeat",
  },
  ".cm-md-done": { color: "var(--o-ink3)", textDecoration: "line-through" },

  ".cm-md-quote": {
    color: "var(--o-ink2)",
    borderLeft: "2px solid var(--o-line2)",
    borderRadius: "0",
    paddingLeft: "12px",
    marginLeft: "0",
  },
  ".cm-md-rule": {
    display: "inline-block",
    width: "100%",
    height: "1px",
    verticalAlign: "middle",
    backgroundColor: "var(--o-line2)",
  },

  ".cm-md-codeblock": {
    fontFamily: "var(--o-font-mono)",
    fontSize: "13px",
    backgroundColor: "var(--o-bg)",
    borderLeft: "1px solid var(--o-line)",
    borderRight: "1px solid var(--o-line)",
    borderRadius: "0",
    margin: "0",
    padding: "0 12px",
  },
  ".cm-md-fence-open": {
    borderTop: "1px solid var(--o-line)",
    borderTopLeftRadius: "10px",
    borderTopRightRadius: "10px",
    marginTop: "8px",
  },
  ".cm-md-fence-close": {
    borderBottom: "1px solid var(--o-line)",
    borderBottomLeftRadius: "10px",
    borderBottomRightRadius: "10px",
    marginBottom: "4px",
  },
  ".cm-md-fence-hidden": { fontSize: "0", lineHeight: "0", paddingBottom: "10px" },
  ".cm-md-lang": {
    display: "inline-block",
    width: "calc(100% + 24px)",
    margin: "0 -12px",
    padding: "0 12px",
    lineHeight: "30px",
    borderBottom: "1px solid var(--o-line)",
    fontFamily: "var(--o-font-sans)",
    fontSize: "12px",
    color: "var(--o-ink3)",
  },

  "&.cm-md-raw .cm-scroller": {
    fontFamily: "var(--o-font-mono)",
    fontSize: "13px",
    lineHeight: "1.75",
  },

  ".cm-md-suggestion": {
    margin: "6px 0",
    padding: "10px 12px",
    borderLeft: "2px solid var(--o-accText)",
    borderRadius: "10px",
    backgroundColor: "var(--o-accSoft)",
  },
  ".cm-md-suggestion-by": {
    fontSize: "12px",
    fontWeight: "600",
    color: "var(--o-accText)",
  },
  ".cm-md-suggestion-text": { whiteSpace: "pre-wrap", margin: "4px 0 8px" },
  ".cm-md-suggestion-actions": { display: "flex", gap: "8px" },
  ".cm-md-suggestion-actions button": {
    height: "28px",
    padding: "0 10px",
    border: "1px solid var(--o-line2)",
    borderRadius: "8px",
    background: "transparent",
    color: "var(--o-ink)",
    font: "inherit",
    fontSize: "13px",
    cursor: "pointer",
  },
  ".cm-md-suggestion-actions button[data-primary=true]": {
    borderColor: "var(--o-acc)",
    backgroundColor: "var(--o-acc)",
    color: "var(--o-white)",
    fontWeight: "600",
  },
});
