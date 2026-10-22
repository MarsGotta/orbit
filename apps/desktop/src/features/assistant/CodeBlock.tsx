import { useEffect, useState, type CSSProperties } from "react";
import { es } from "../../i18n/es";
import { CopyButton } from "./CopyButton";
import type { CodeToken } from "./highlight";
import styles from "./ChatMarkdown.module.css";

/**
 * The coloured lines of `code` once the highlighter has them. Shiki and the
 * grammar load only here, the first time a block names a language; until then,
 * and for a language it does not know, the code is plain text.
 */
function useHighlight(code: string, language: string | null): CodeToken[][] | null {
  // The lines are kept with the code they colour: while an answer streams,
  // older colours never stand for newer text.
  const [coloured, setColoured] = useState<{ code: string; lines: CodeToken[][] | null }>();
  useEffect(() => {
    if (language === null) return;
    let current = true;
    void import("./highlight")
      .then(({ highlight }) => highlight(code, language))
      .then((tokens) => {
        if (current) setColoured({ code, lines: tokens });
      })
      .catch(() => undefined);
    return () => {
      current = false;
    };
  }, [code, language]);
  return coloured?.code === code ? coloured.lines : null;
}

function Lines({ lines }: { lines: CodeToken[][] }) {
  return lines.map((line, index) => (
    <span key={index}>
      {index > 0 && "\n"}
      {line.map((token, at) => (
        <span key={at} style={token.style as CSSProperties}>
          {token.content}
        </span>
      ))}
    </span>
  ));
}

/**
 * A fenced block: its language, «Copiar» and the code, coloured when
 * `colour` (not while the answer still streams, so colours do not blink at
 * every new piece of text).
 */
export function CodeBlock({
  code,
  colour,
  language,
}: {
  code: string;
  colour: boolean;
  language: string | null;
}) {
  const lines = useHighlight(code, colour ? language : null);
  return (
    <figure className={styles.code}>
      <figcaption data-speech="skip">
        <span>{language}</span>
        <CopyButton label={es.chatActions.copyCode} text={() => code} />
      </figcaption>
      <pre>
        <code>{lines === null ? code : <Lines lines={lines} />}</code>
      </pre>
    </figure>
  );
}
