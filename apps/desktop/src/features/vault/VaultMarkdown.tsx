import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { VaultMeta } from "../../bridge/generated/VaultMeta";
import type { VaultNote } from "../../bridge/generated/VaultNote";
import { TextLink } from "../../design/TextLink";
import type { VaultPort } from "./api";
import { internalTarget, remarkVault } from "./markdown-tree";
import { noteRoute } from "./route";
import { TaskRows } from "./TaskRows";
import { DependencyDiagram } from "./DependencyDiagram";
import { VaultTasks } from "./VaultTasks";
import styles from "./VaultReading.module.css";

interface Props {
  note: VaultNote;
  notes: VaultMeta[];
  port?: VaultPort;
}
function SourceLink({
  href,
  note,
  children,
}: {
  href: string;
  note: VaultNote;
  children: React.ReactNode;
}) {
  const citation = note.citations.find(
    (source) => `#orbit-source-${encodeURIComponent(source.id)}` === href,
  );
  if (citation !== undefined)
    return (
      <sup>
        <a
          className={styles.citation}
          href={`#vault-source-${encodeURIComponent(citation.id)}`}
          onClick={(event) => {
            event.preventDefault();
            document
              .getElementById(`vault-source-${citation.id}`)
              ?.scrollIntoView({ block: "nearest" });
          }}
          title={citation.title}
        >
          {children}
        </a>
      </sup>
    );
  const index = /^#orbit-note-(\d+)$/.exec(href)?.[1];
  const path =
    index !== undefined ? note.outlinks[Number(index)]?.path : internalTarget(note.path, href);
  if (path != null) return <TextLink href={noteRoute(path)}>{children}</TextLink>;
  if (/^https?:\/\//i.test(href))
    return (
      <TextLink href={href} rel="noopener noreferrer" target="_blank">
        {children}
      </TextLink>
    );
  if (href.startsWith("#^"))
    return (
      <a
        href={`#vault-task-${href.slice(2)}`}
        onClick={(event) => {
          event.preventDefault();
          document
            .getElementById(`vault-task-${href.slice(2)}`)
            ?.scrollIntoView({ block: "nearest" });
        }}
      >
        {children}
      </a>
    );
  return <span>{children}</span>;
}
function CodeBlock({
  className,
  children,
  taskRange,
  ...props
}: Props & { className?: string; children: React.ReactNode; taskRange?: unknown }) {
  const language = className?.replace("language-", "");
  if (language === "dependencias") return <DependencyDiagram note={props.note} />;
  if (language === "tareas") {
    const text = String(children);
    const person = /^persona:\s*(.+)$/m.exec(text)?.[1] ?? "";
    const project = /^proyecto:\s*(.+)$/m.exec(text)?.[1];
    const path =
      project === undefined ? "" : (props.notes.find((n) => n.title === project)?.path ?? "");
    if (project !== undefined && path === "")
      return <p>El proyecto de esta consulta no está en la página del índice: {project}.</p>;
    const state = /^estado:\s*(.+)$/m.exec(text)?.[1];
    return (
      <VaultTasks
        embedded
        initialState={state === "abiertas" ? "open" : state === "hechas" ? "done" : "all"}
        notes={props.notes}
        path={path}
        port={props.port}
        query={person}
      />
    );
  }
  // Only our AST transform supplies this attribute. A note's ordinary
  // code fences must stay literal, including a fence named "orbit-tasks".
  const range =
    typeof taskRange === "string" ? /^\{"from":(\d+),"to":(\d+)\}$/.exec(taskRange) : null;
  if (language === "orbit-tasks" && range !== null) {
    return (
      <TaskRows
        notes={props.notes}
        tasks={props.note.tasks.filter(
          (task) => task.line >= Number(range[1]) && task.line <= Number(range[2]),
        )}
      />
    );
  }
  return <code className={className}>{children}</code>;
}
export function VaultMarkdown(props: Props) {
  return (
    <div className={styles.markdown}>
      <ReactMarkdown
        components={{
          a: ({ href, children }) => (
            <SourceLink href={href ?? ""} note={props.note}>
              {children}
            </SourceLink>
          ),
          img: ({ alt }) => <span>{alt}</span>,
          input: ({ checked }) => (
            <input
              aria-label="Tarea de solo lectura"
              checked={checked}
              disabled
              readOnly
              type="checkbox"
            />
          ),
          pre: ({ children }) => <div className={styles.codeBlock}>{children}</div>,
          code: ({ className, children, node }) => (
            <CodeBlock
              {...props}
              className={className}
              taskRange={node?.properties["data-orbit-task-range"]}
            >
              {children}
            </CodeBlock>
          ),
        }}
        remarkPlugins={[remarkGfm, remarkVault(props.note)]}
        skipHtml
      >
        {props.note.body}
      </ReactMarkdown>
    </div>
  );
}
