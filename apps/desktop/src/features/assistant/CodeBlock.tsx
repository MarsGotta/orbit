import { Fragment, useEffect, useState, type CSSProperties } from "react";
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

/** The names a reader knows a language by; any other is shown as the fence wrote it. */
const LANGUAGE_NAMES: Record<string, string> = {
  bash: "Bash",
  c: "C",
  cpp: "C++",
  css: "CSS",
  csv: "CSV",
  go: "Go",
  html: "HTML",
  java: "Java",
  javascript: "JavaScript",
  js: "JavaScript",
  json: "JSON",
  jsx: "JSX",
  kotlin: "Kotlin",
  markdown: "Markdown",
  md: "Markdown",
  py: "Python",
  python: "Python",
  rb: "Ruby",
  ruby: "Ruby",
  rs: "Rust",
  rust: "Rust",
  sh: "Shell",
  shell: "Shell",
  sql: "SQL",
  swift: "Swift",
  toml: "TOML",
  ts: "TypeScript",
  tsx: "TSX",
  typescript: "TypeScript",
  xml: "XML",
  yaml: "YAML",
  yml: "YAML",
  zsh: "Zsh",
};

/**
 * What a fence's info string says: its language and the file the code is
 * from, as agents write it (`python exportar.py`, `python:exportar.py` or
 * `python title="exportar.py"`).
 */
export function fenceInfo(
  language: string | null,
  meta: string | null,
): { language: string | null; file: string | null } {
  const colon = language?.indexOf(":") ?? -1;
  const name = language === null || colon < 0 ? language : language.slice(0, colon);
  const colonFile = language !== null && colon >= 0 ? language.slice(colon + 1) : "";
  const titled = (meta === null ? null : /(?:title|file|filename)=["']?([^"'\s]+)/.exec(meta))?.[1];
  const first = meta?.trim().split(/\s+/)[0] ?? "";
  const file = colonFile !== "" ? colonFile : (titled ?? (first.includes("=") ? "" : first));
  return { language: name === "" ? null : name, file: file === "" ? null : file };
}

const SHELLS = new Set(["sh", "bash", "zsh", "shell", "console", "terminal"]);

/**
 * The commands of a shell block for the person to run, each without its
 * `$`: a block of one line, or one where every line starts with `$`. Any
 * other (a script, a command and its output) is code, and `null` here.
 */
export function shellCommands(language: string | null, code: string): string[] | null {
  if (language === null || !SHELLS.has(language.toLowerCase())) return null;
  const lines = code.split("\n").filter((line) => line.trim() !== "");
  if (lines.length === 0) return null;
  if (lines.every((line) => /^\s*\$(\s|$)/.test(line)))
    return lines.map((line) => line.replace(/^\s*\$\s?/, ""));
  return lines.length === 1 ? lines.map((line) => line.trim()) : null;
}

/** A command for the person to run: a `$` line each, and «Copiar comando» without the `$`. */
function CommandBlock({ commands }: { commands: string[] }) {
  return (
    <figure aria-label={es.chatActions.command} className={styles.command}>
      <pre>
        <code>
          {commands.map((command, index) => (
            <Fragment key={index}>
              {index > 0 && "\n"}
              <span aria-hidden="true" className={styles.prompt} data-speech="skip">
                $
              </span>
              <span>{command}</span>
            </Fragment>
          ))}
        </code>
      </pre>
      <span data-speech="skip">
        <CopyButton
          label={es.chatActions.copyCommand}
          text={() => commands.join("\n")}
          tip={es.chatActions.copyCommand}
        />
      </span>
    </figure>
  );
}

/**
 * A fenced block: its language and the file it names over the code, with
 * «Copiar código», coloured when `colour` (not while the answer still
 * streams, so colours do not blink at every new piece of text). A shell
 * command for the person to run is a `$` line instead.
 */
export function CodeBlock({
  code,
  colour,
  language,
  meta = null,
}: {
  code: string;
  colour: boolean;
  language: string | null;
  meta?: string | null;
}) {
  const info = fenceInfo(language, meta);
  const lines = useHighlight(code, colour ? info.language : null);
  const commands = info.file === null ? shellCommands(info.language, code) : null;
  if (commands !== null) return <CommandBlock commands={commands} />;
  const name = info.language === null ? null : (LANGUAGE_NAMES[info.language] ?? info.language);
  return (
    <figure className={styles.code}>
      <figcaption data-speech="skip">
        {name !== null && <span>{name}</span>}
        {name !== null && info.file !== null && <span aria-hidden="true">·</span>}
        {info.file !== null && <span className={styles.file}>{info.file}</span>}
        <span className={styles.spacer} />
        <CopyButton
          label={es.chatActions.copyCode}
          text={() => code}
          tip={es.chatActions.copyCode}
        />
      </figcaption>
      <pre>
        <code>{lines === null ? code : <Lines lines={lines} />}</code>
      </pre>
    </figure>
  );
}
