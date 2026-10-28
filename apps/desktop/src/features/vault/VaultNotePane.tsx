import { useCopy } from "../../i18n/CopyProvider";
import { useEffect } from "react";
import type { VaultCatalog } from "../../bridge/generated/VaultCatalog";
import type { VaultNote } from "../../bridge/generated/VaultNote";
import { TopBarLeading, Breadcrumbs } from "../../app/shell-slots";
import { Button } from "../../design/Button";

import { readFilter, type VaultPort } from "./api";
import { AboutNote } from "./AboutNote";
import { typeLabel } from "./type-colors";
import { DependencyDiagram } from "./DependencyDiagram";
import { NoteProperties } from "./NoteProperties";
import { noteRoute, type VaultRoute } from "./route";
import { useVaultRead } from "./use-vault-read";
import { VaultMarkdown } from "./VaultMarkdown";
import styles from "./VaultReading.module.css";

function useMarkdownShortcut(path: string, markdown: boolean) {
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key === "/") {
        event.preventDefault();
        window.location.hash = noteRoute(path, !markdown);
      }
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [path, markdown]);
}
function RawNote({ note }: { note: VaultNote }) {
  const inlineCopy = useCopy().vaultReading;
  const copy = useCopy().extras.vaultCopy;
  const prefix = note.raw.endsWith(note.body)
    ? note.raw.slice(0, note.raw.length - note.body.length)
    : "";
  return (
    <div className={styles.raw}>
      <p className={styles.muted}>
        {copy.yaml} {inlineCopy.readOnly}
      </p>
      {prefix !== "" && <pre>{prefix}</pre>}
      <pre>{note.body}</pre>
    </div>
  );
}
function NoteBar({ note, title, markdown }: { note: VaultNote; title: string; markdown: boolean }) {
  const inlineCopy = useCopy().vaultReading;
  const copy = useCopy().extras.vaultCopy;
  return (
    <TopBarLeading>
      <div className={styles.top}>
        <Breadcrumbs
          trail={[
            {
              label:
                copy.folderNames[note.path.split("/")[0] as keyof typeof copy.folderNames] ??
                inlineCopy.notesTitle,
              href: "#/vault",
            },
            { label: title },
          ]}
        />
        <code title={note.path}>/{note.path}</code>
        <span>{inlineCopy.readFromHub}</span>
        <Button
          onClick={() => {
            window.location.hash = noteRoute(note.path, !markdown);
          }}
          size={28}
        >
          {markdown ? copy.text : inlineCopy.markdownToggle(copy.md)}
        </Button>
      </div>
    </TopBarLeading>
  );
}

function ReadNote({
  note,
  catalog,
  port,
  markdown,
}: {
  note: VaultNote;
  catalog: VaultCatalog;
  port?: VaultPort;
  markdown: boolean;
}) {
  const copy = useCopy().extras.vaultCopy;
  const title =
    typeof note.frontmatter.title === "string"
      ? note.frontmatter.title
      : (note.path.split("/").pop() ?? note.path);
  const type = note.type;
  return (
    <>
      <NoteBar note={note} title={title} markdown={markdown} />
      <div className={styles.noteLayout}>
        <article className={styles.noteContent}>
          <span className={styles.type} data-type={note.role}>
            {typeLabel(type, note.role)}
          </span>
          <h1>{title}</h1>
          {typeof note.properties.description === "string" && (
            <p className={styles.description}>{note.properties.description}</p>
          )}
          <NoteProperties note={note} notes={catalog.notes} fieldNames={catalog.schema.fields} />
          {note.schema_warnings.map((warning) => (
            <p role="alert" key={warning}>
              {warning}
            </p>
          ))}
          {markdown ? (
            <RawNote note={note} />
          ) : (
            <>
              <VaultMarkdown note={note} notes={catalog.notes} port={port} />
              {note.role === "project" && <DependencyDiagram note={note} />}
            </>
          )}
          <p className={styles.muted}>{copy.readOnly}</p>
        </article>
        <AboutNote catalog={catalog} note={note} port={port} />
      </div>
    </>
  );
}
export function VaultNotePane({
  port,
  route,
  catalog,
}: {
  port?: VaultPort;
  route: VaultRoute;
  catalog: VaultCatalog;
}) {
  const inlineCopy = useCopy().vaultReading;
  useMarkdownShortcut(route.path, route.markdown);
  const { answer, error, retry } = useVaultRead(port, readFilter("read", route.path));
  if (error !== null)
    return (
      <p className={styles.content} role="alert">
        {error}{" "}
        <Button onClick={retry} size={28}>
          {inlineCopy.retry}
        </Button>
      </p>
    );
  if (answer?.outcome !== "note")
    return (
      <p className={styles.content} role="status">
        {inlineCopy.readingNote}
      </p>
    );
  return <ReadNote catalog={catalog} markdown={route.markdown} note={answer.note} port={port} />;
}
