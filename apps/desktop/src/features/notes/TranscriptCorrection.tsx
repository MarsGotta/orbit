import { useRef, useState, type ReactNode } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import { CorrectionDialog } from "./CorrectionDialog";

export function TranscriptCorrection({
  note,
  library,
  send,
  children,
}: {
  note: OpenNoteSnapshot;
  library?: NoteLibrarySnapshot | null;
  send: (command: AppCommand) => void;
  children: ReactNode;
}) {
  const [text, setText] = useState("");
  const [open, setOpen] = useState(false);
  const transcript = useRef<HTMLDivElement>(null);
  const capture = () => {
    const selection = window.getSelection();
    if (!selection || selection.rangeCount !== 1 || selection.isCollapsed) {
      setText("");
      return;
    }
    const range = selection.getRangeAt(0);
    const start =
      range.startContainer.nodeType === Node.ELEMENT_NODE
        ? (range.startContainer as Element)
        : range.startContainer.parentElement;
    const paragraph = start?.closest("article p");
    const inside =
      paragraph != null &&
      transcript.current?.contains(paragraph) === true &&
      paragraph.contains(range.endContainer);
    setText(inside ? selection.toString().trim() : "");
  };
  return (
    <>
      {text && (
        <Button size={28} variant="ghost" onClick={() => setOpen(true)}>
          {es.dictionary.correctText}
        </Button>
      )}
      {open && (
        <CorrectionDialog
          wrong={text}
          note={note}
          library={library}
          send={send}
          onClose={() => {
            setOpen(false);
            setText("");
          }}
        />
      )}
      <div ref={transcript} onMouseUp={capture} onKeyUp={capture}>
        {children}
      </div>
    </>
  );
}
