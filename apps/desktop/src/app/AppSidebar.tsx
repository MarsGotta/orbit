import type { AppCommand } from "../bridge/generated";
import { AssistantSessions } from "../features/assistant/AssistantSessions";
import type { ViewId } from "../state/ui";
import type { AppContentProps } from "./App";
import { ContextPanel, contextForView } from "./ContextPanel";
import { Rail } from "./Rail";

export function AppSidebar({
  props,
  view,
  dispatch,
}: {
  props: AppContentProps;
  view: ViewId;
  dispatch: (command: AppCommand) => void;
}) {
  const panel = contextForView(view, props.snapshot, dispatch);
  return (
    <>
      <Rail activeView={view} onNavigate={props.navigate} snapshot={props.snapshot} />
      {view === "assistant" ? (
        <AssistantSessions
          read={props.readSessionsPage}
          send={dispatch}
          snapshot={props.snapshot}
        />
      ) : panel ? (
        <ContextPanel data={panel} narrow={view === "notes"} />
      ) : null}
    </>
  );
}
