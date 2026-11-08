import { useCopy } from "../../i18n/CopyProvider";
import { useState } from "react";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";
import { ConfirmCard } from "../../design/ConfirmCard";
import { Icon } from "../../design/Icon";

import { CopyButton } from "./copy-button";
import { isPreparing, isRecording, templateOf } from "./note-progress";
import { summaryMarkdown } from "./notes-format";
import type { Rewriting } from "./use-rewriting";
import styles from "./Redo.module.css";

/**
 * «Copiar resumen» and «Rehacer resumen», above the summary (annex, change 7).
 * Writing it again asks first and says the hand-edited version is kept: the
 * hub keeps it as `superseded_edits`, and «Recuperar mi versión» brings it back.
 */
export function SummaryTools({
  note,
  rewriting,
}: {
  note: OpenNoteSnapshot;
  rewriting: Rewriting;
}) {
  const es = useCopy();
  const [anchor, setAnchor] = useState<HTMLButtonElement | null>(null);
  const [asking, setAsking] = useState(false);
  const blocked = isRecording(note.progress) ? es.noteScreen.whileRecording : undefined;
  const template = note.summary?.template_name ?? templateOf(note);
  return (
    <div className={styles.tools}>
      {note.summary && (
        <CopyButton label={es.noteScreen.copySummary} text={() => summaryMarkdown(note)} />
      )}
      {!isPreparing(note) && (
        <Button
          aria-expanded={asking}
          disabled={rewriting.template !== null}
          disabledReason={blocked}
          onClick={() => setAsking(!asking)}
          ref={setAnchor}
          size={28}
          variant="outline"
        >
          <Icon name="refresh" size={14} />
          {es.noteScreen.redoSummary}
        </Button>
      )}
      {asking && (
        <ConfirmCard
          anchor={anchor}
          confirmLabel={es.noteScreen.redoSummary}
          onClose={() => setAsking(false)}
          onConfirm={() => {
            setAsking(false);
            rewriting.redo();
          }}
          title={es.noteScreen.redoSummaryTitle}
        >
          {es.noteScreen.redoSummaryBody(template)}
        </ConfirmCard>
      )}
    </div>
  );
}
