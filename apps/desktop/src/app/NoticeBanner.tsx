import { Toast } from "../design/Toast";
import type { NoticeAction } from "../i18n/es/notices";
import type { Message } from "./interceptor";
export interface NoticeBannerProps {
  messages: Message[];
  onAction: (action: NoticeAction) => void;
  onDismiss: (op: string) => void;
}

/** Floating messages, one per op; problems remain until the person closes them. */
export function NoticeBanner({ messages, onAction, onDismiss }: NoticeBannerProps) {
  return (
    <>
      {messages.map(({ op, class: kind, text, action }) => (
        <Toast
          key={op}
          kind={kind}
          message={text}
          onClose={() => onDismiss(op)}
          action={
            action && (
              <button onClick={() => onAction(action)} type="button">
                {action.label}
              </button>
            )
          }
        />
      ))}
    </>
  );
}
