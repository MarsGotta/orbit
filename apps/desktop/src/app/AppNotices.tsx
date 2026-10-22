import { useCallback } from "react";
import type { NoticeAction } from "../i18n/es/notices";
import type { AppContentProps } from "./App";
import { ToastStack } from "../design/Toast";
import { UpdateNotice } from "./UpdateNotice";
import { NoticeBanner } from "./NoticeBanner";
import { bannerMessages } from "./visible-error";

type Props = Pick<
  AppContentProps,
  "dismiss" | "messages" | "navigate" | "retry" | "send" | "snapshot"
>;

/** Shared floating corner for the interceptor and local confirmations. */
export function AppNotices({ dismiss, messages, navigate, retry, send, snapshot }: Props) {
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
      <NoticeBanner
        messages={bannerMessages(messages)}
        onAction={onAction}
        onDismiss={dismiss}
        onRetry={retry}
      />
    </>
  );
}
