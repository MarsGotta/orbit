import { useCopy } from "../../i18n/CopyProvider";
import { createContext, memo, useContext, type ComponentProps } from "react";
import Markdown, { type Components, type ExtraProps, type Options } from "react-markdown";
import remarkGfm from "remark-gfm";
import { TextLink } from "../../design/TextLink";

import { safeHref } from "../notes/markdown";
import { CodeBlock } from "./CodeBlock";
import { CopyButton } from "./CopyButton";
import styles from "./ChatMarkdown.module.css";

type HastNode = NonNullable<ExtraProps["node"]>;

interface MdNode {
  type: string;
  value?: string;
  children?: MdNode[];
}

const FLOW_PARENTS = new Set(["root", "blockquote", "listItem"]);

/**
 * Raw HTML in an answer is shown as the text it is: a block of it becomes a
 * paragraph of text, inline HTML a run of text. No element of the model's
 * ever reaches the page.
 */
function htmlAsText() {
  const walk = (node: MdNode) => {
    node.children = node.children?.map((child) => {
      if (child.type !== "html") {
        walk(child);
        return child;
      }
      const text = { type: "text", value: child.value ?? "" };
      return FLOW_PARENTS.has(node.type) ? { type: "paragraph", children: [text] } : text;
    });
  };
  return walk;
}

const PLUGINS: NonNullable<Options["remarkPlugins"]> = [remarkGfm, htmlAsText];

/** The Orbit view a link opens, when it is one of Orbit's own routes. */
function orbitView(href: string): "note" | "calendar" | null {
  if (/^#\/notes\/[^/\s]+$/.test(href)) return "note";
  if (href === "#/calendar") return "calendar";
  return null;
}

/** Only web links and Orbit's own views stay links; anything else is dropped. */
function urlTransform(url: string): string {
  return safeHref(url) ?? (orbitView(url) === null ? "" : url);
}

function textOf(node: HastNode | HastNode["children"][number]): string {
  if (node.type === "text") return node.value;
  if (node.type === "element") return node.children.map(textOf).join("");
  return "";
}

function codeOf(node: HastNode | undefined): {
  code: string;
  language: string | null;
  meta: string | null;
} {
  const code = node?.children.find((child) => child.type === "element" && child.tagName === "code");
  if (code?.type !== "element") return { code: "", language: null, meta: null };
  const classes = code.properties.className;
  const language = Array.isArray(classes)
    ? classes.map(String).find((name) => name.startsWith("language-"))
    : undefined;
  // What follows the language on the fence's line (its file name), as the parser keeps it.
  const meta = (code.data as { meta?: unknown } | undefined)?.meta;
  // The fence's last newline is the parser's, not the code's.
  return {
    code: textOf(code).replace(/\n$/, ""),
    language: language === undefined ? null : language.slice("language-".length),
    meta: typeof meta === "string" ? meta : null,
  };
}

/**
 * The answer being drawn, for the blocks that need it. Through a context and
 * not a closure: `COMPONENTS` stays the same object across renders, so React
 * keeps each block (its colours, its «Copiado») instead of mounting it again.
 */
const Answer = createContext({ source: "", streaming: false });

function Pre({ node }: ExtraProps) {
  const { streaming } = useContext(Answer);
  const { code, language, meta } = codeOf(node);
  return <CodeBlock code={code} colour={!streaming} language={language} meta={meta} />;
}

function Table({ children, node }: ComponentProps<"table"> & ExtraProps) {
  const es = useCopy();
  const { source } = useContext(Answer);
  const start = node?.position?.start.offset;
  const end = node?.position?.end.offset;
  return (
    <div className={styles.table}>
      <div data-scroll="x">
        <table>{children}</table>
      </div>
      {start !== undefined && end !== undefined && (
        <div data-speech="skip">
          <CopyButton label={es.chatActions.copyTable} text={() => source.slice(start, end)} />
        </div>
      )}
    </div>
  );
}

/**
 * A table cell that says only a state is coloured by it, as the design's
 * tables are: ready in green, waiting in amber, stopped in red. Any other
 * cell is plain text: «Hecha» in a column of dates is not a state.
 */
const STATES: Record<string, "ok" | "warn" | "rec"> = {
  listo: "ok",
  lista: "ok",
  pendiente: "warn",
  "en curso": "warn",
  bloqueado: "rec",
  bloqueada: "rec",
  fallido: "rec",
  fallida: "rec",
};

function Cell({ children, node, ...props }: ComponentProps<"td"> & ExtraProps) {
  const state = node === undefined ? undefined : STATES[textOf(node).trim().toLowerCase()];
  return (
    <td {...props} data-state={state}>
      {children}
    </td>
  );
}

const COMPONENTS: Components = {
  a: ({ children, href }) => {
    const view = href === undefined ? null : orbitView(href);
    if (view !== null)
      return (
        <a className={styles.chip} data-chip={view} href={href}>
          {children}
        </a>
      );
    if (href === undefined || href === "") return <span>{children}</span>;
    return (
      <TextLink href={href} rel="noopener noreferrer" target="_blank">
        {children}
      </TextLink>
    );
  },
  // An image would be fetched from wherever the model points; its text is enough.
  img: ({ alt }) => <span>{alt}</span>,
  h1: "h3",
  h2: "h4",
  h3: "h5",
  h4: "h6",
  h5: "h6",
  pre: Pre,
  table: Table,
  td: Cell,
};

/**
 * An answer's Markdown, GFM included: code blocks with their language, the
 * file they name and «Copiar» (coloured once complete), a shell command as a
 * `$` line copied without it, tables that scroll and copy as Markdown,
 * quotes, rules and inline code. Text is text: no HTML from the model, only
 * web links and Orbit's own views are links, and no image is loaded.
 */
export const ChatMarkdown = memo(function ChatMarkdown({
  source,
  streaming = false,
}: {
  source: string;
  streaming?: boolean;
}) {
  return (
    <div className={styles.markdown}>
      <Answer value={{ source, streaming }}>
        <Markdown components={COMPONENTS} remarkPlugins={PLUGINS} urlTransform={urlTransform}>
          {source}
        </Markdown>
      </Answer>
    </div>
  );
});
