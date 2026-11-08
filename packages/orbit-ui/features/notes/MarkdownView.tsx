import type { ReactNode } from "react";
import { TextLink } from "../../design/TextLink";
import { type Block, type Run, parseMarkdown } from "./markdown";
import styles from "./MarkdownView.module.css";

/** How a link is drawn where the default (a link that opens in the browser) is not enough. */
export type LinkRenderer = (href: string, text: string) => ReactNode;

function renderRun(run: Run, key: number, link?: LinkRenderer): ReactNode {
  let node: ReactNode = run.text
    .split("\n")
    .flatMap((line, index) => (index === 0 ? [line] : [<br key={index} />, line]));
  if (run.italic === true) node = <em>{node}</em>;
  if (run.bold === true) node = <strong>{node}</strong>;
  if (run.strike === true) node = <s>{node}</s>;
  if (run.href !== undefined && link !== undefined) {
    node = link(run.href, run.text);
  } else if (run.href !== undefined) {
    node = (
      <TextLink href={run.href} rel="noopener noreferrer" target="_blank">
        {node}
      </TextLink>
    );
  }
  return <span key={key}>{node}</span>;
}

function renderBlock(block: Block, key: number, link?: LinkRenderer): ReactNode {
  const runs = (items: Run[]) => items.map((run, index) => renderRun(run, index, link));
  switch (block.kind) {
    case "paragraph":
      return <p key={key}>{runs(block.runs)}</p>;
    case "heading": {
      // Inside a note the page already has its h1 and the section its h2.
      const Heading = (["h3", "h4", "h5"] as const)[block.level - 1] ?? "h5";
      return <Heading key={key}>{runs(block.runs)}</Heading>;
    }
    case "bullet":
      return (
        <ul key={key}>
          {block.items.map((item, index) => (
            <li key={index}>{runs(item)}</li>
          ))}
        </ul>
      );
    case "ordered":
      return (
        <ol key={key} start={block.start}>
          {block.items.map((item, index) => (
            <li key={index}>{runs(item)}</li>
          ))}
        </ol>
      );
    case "tasks":
      return (
        <ul className={styles.tasks} key={key}>
          {block.items.map((item, index) => (
            <li data-checked={item.checked} key={index}>
              <span aria-hidden="true" className={styles.box} />
              <span>{runs(item.runs)}</span>
            </li>
          ))}
        </ul>
      );
  }
}

/**
 * Markdown of the note's subset as React nodes. Text is text: no HTML from
 * the model reaches the page, and only http(s) links are links.
 */
export function MarkdownView({ link, source }: { link?: LinkRenderer; source: string }) {
  return (
    <div className={styles.markdown}>
      {parseMarkdown(source).map((block, key) => renderBlock(block, key, link))}
    </div>
  );
}
