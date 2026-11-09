import { useCopy } from "../../i18n/CopyProvider";
import { Button } from "../../design/Button";
import { useFeedback } from "../../design/feedback";

/**
 * Every «Copiar» of the note says «✓ Copiado» once the text is on the
 * clipboard (proposal 11), and goes back to its label after 1.8 s.
 */
export function CopyButton({
  label,
  size = 28,
  text,
}: {
  label: string;
  size?: 28 | 36;
  text: () => string;
}) {
  const es = useCopy();
  const { feedback, track } = useFeedback();
  return (
    <Button
      doneLabel={es.notes.copied}
      feedback={feedback}
      onClick={() => {
        const clipboard = navigator.clipboard as Clipboard | undefined;
        if (clipboard) track(clipboard.writeText(text()));
      }}
      pendingLabel={label}
      size={size}
      variant="ghost"
    >
      {label}
    </Button>
  );
}
