import { useCallback } from "react";
import type { NoticeAction } from "../i18n";
import type { AppContentProps } from "./App";
import { ToastStack } from "../design/Toast";
import { MeetingNoteNotice } from "../features/notes/MeetingNoteNotice";
import { UnseenWriteOutcomes } from "../features/calendar/edit/UnseenWriteOutcomes";
import { UpdateNotice } from "./UpdateNotice";
import { NoticeBanner } from "./NoticeBanner";
import { bannerMessages } from "./visible-error";

type Props = Pick<
  AppContentProps,
  "dismiss" | "messages" | "navigate" | "retry" | "send" | "snapshot" | "view"
>;

/** Shared floating corner for the interceptor and local confirmations. */
export function AppNotices({ dismiss, messages, navigate, retry, send, snapshot, view }: Props) {
  const onAction = useCallback(
    (action: NoticeAction) => {
      if ("view" in action) navigate(action.view);
      else void send(action.command);
    },
    [navigate, send],
  );
  return (
    <>
      <ToastStack />
      <UpdateNotice update={snapshot.settings.update} send={send} />
      <MeetingNoteNotice meeting={snapshot.meeting} notes={snapshot.notes} send={send} />
      <UnseenWriteOutcomes
        calendarOpen={view === "calendar"}
        onOpenCalendar={() => navigate("calendar")}
        writes={snapshot.calendar?.writes}
      />
      <NoticeBanner
        messages={bannerMessages(messages)}
        onAction={onAction}
        onDismiss={dismiss}
        onRetry={retry}
      />
    </>
  );
}
