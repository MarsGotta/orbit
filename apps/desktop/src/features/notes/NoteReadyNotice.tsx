import { useEffect, useRef, useState } from "react";
import type { NoteReadySnapshot } from "../../bridge/generated/NoteReadySnapshot";
import { Button } from "../../design/Button";
import { Toast } from "../../design/Toast";
import { es } from "../../i18n/es";
import { openMeeting } from "./note-route";
import type { Send } from "./send";

/**
 * «Tu nota está lista · Abrir» (proposal 7), apart from «Reunión guardada»:
 * once for each note the app saw finish while this window looked. The note
 * ready before it looked says nothing.
 */
export function NoteReadyNotice({ ready, send }: { ready?: NoteReadySnapshot; send: Send }) {
  const seen = useRef(ready?.sequence ?? 0);
  const [shown, setShown] = useState<NoteReadySnapshot | null>(null);
  useEffect(() => {
    if (ready && ready.sequence > seen.current) {
      seen.current = ready.sequence;
      setShown(ready);
    }
  }, [ready]);
  if (shown === null) return null;
  return (
    <Toast
      action={
        <Button
          onClick={() => {
            setShown(null);
            openMeeting(shown.recording_id, send);
          }}
          size={28}
          variant="outline"
        >
          {es.noteScreen.open}
        </Button>
      }
      detail={shown.title}
      key={shown.sequence}
      message={es.noteScreen.ready}
      onClose={() => setShown(null)}
    />
  );
}
