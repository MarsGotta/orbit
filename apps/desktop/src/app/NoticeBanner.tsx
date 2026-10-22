import { Toast } from "../design/Toast";
import type { NoticeAction } from "../i18n/es/notices";
import { FailureActions, FailureHelp } from "./FailureActions";
import type { Message } from "./interceptor";
export interface NoticeBannerProps {
  messages: Message[];
  onAction: (action: NoticeAction) => void;
  onDismiss: (op: string) => void;
  /** «Reintentar» on a failure whose order can be sent again. */
  onRetry: (op: string) => void;
}

/** Floating messages, one per op; problems remain until the person closes them. */
export function NoticeBanner({ messages, onAction, onDismiss, onRetry }: NoticeBannerProps) {
  return (
    <>
      {messages.map(({ op, class: kind, text, action, failure }) => (
        <Toast
          key={op}
          kind={kind}
          // The generic failure reads as its title, then what to do and the code.
          message={failure?.title ?? text}
          detail={failure?.title !== undefined && <FailureHelp code={failure.code} />}
          onClose={() => onDismiss(op)}
          action={
            failure ? (
              <FailureActions failure={failure} onRetry={onRetry} op={op} />
            ) : (
              action && (
                <button onClick={() => onAction(action)} type="button">
                  {action.label}
                </button>
              )
            )
          }
        />
      ))}
    </>
  );
}
