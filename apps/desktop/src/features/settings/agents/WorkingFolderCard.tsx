import { useId, useState } from "react";
import { Button } from "../../../design/Button";
import { Field } from "../../../design/Field";
import { Group } from "../layout";
import { copy, FeedbackLine, type OrderProps } from "./parts";

/** The hub validates this path; the Mac never opens or inspects it. */
export function WorkingFolderCard({
  path,
  agentId,
  busy,
  feedback,
  order,
}: OrderProps & { path: string }) {
  const [draft, setDraft] = useState(path);
  const errorId = useId();
  const refused = feedback?.place === "working_folder" && feedback.kind === "error";
  return (
    <Group help={copy.workingFolderHelp} label={copy.workingFolder}>
      <Field
        aria-describedby={refused ? errorId : undefined}
        aria-invalid={refused || undefined}
        autoCapitalize="none"
        autoComplete="off"
        disabled={busy}
        label={copy.workingFolderLabel}
        onChange={(event) => setDraft(event.currentTarget.value)}
        spellCheck={false}
        value={draft}
      />
      <div>
        <Button
          disabled={busy || draft === path}
          onClick={() =>
            void order({ call: "working_folder", agent_id: agentId, path: draft }, "working_folder")
          }
          size={28}
        >
          {copy.saveWorkingFolder}
        </Button>
      </div>
      <div id={errorId}>
        <FeedbackLine feedback={feedback} place="working_folder" />
      </div>
    </Group>
  );
}
