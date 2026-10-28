import { useCopy } from "../i18n/CopyProvider";
import { useState } from "react";
import type { AppCommand } from "../bridge/generated";
import { Popover } from "../design/Popover";
import { Spinner } from "../design/Spinner";

import type { NoticeFix, TopBarNotice } from "./status-problem";
import styles from "./TopBar.module.css";

export interface NoticeActions {
  send: (command: AppCommand) => Promise<unknown>;
  onPair: () => void;
}

/** The button that fixes it: a section of Ajustes, an order, or pairing. */
function Fix({ fix, actions }: { fix: NoticeFix; actions: NoticeActions }) {
  const [pending, setPending] = useState(false);
  if (fix.kind === "route")
    return (
      <a className={styles.fix} href={fix.route}>
        {fix.label}
      </a>
    );
  if (fix.kind === "pair")
    return (
      <button className={styles.fix} onClick={actions.onPair} type="button">
        {fix.label}
      </button>
    );
  // Until Orbit answers the order: the state that follows takes the notice away.
  if (pending)
    return (
      <span className={styles.working}>
        <Spinner size={12} />
        {fix.pending}
      </span>
    );
  return (
    <button
      className={styles.fix}
      onClick={() => {
        setPending(true);
        void actions.send(fix.command).finally(() => setPending(false));
      }}
      type="button"
    >
      {fix.label}
    </button>
  );
}

function NoticeRow({ notice, actions }: { notice: TopBarNotice; actions: NoticeActions }) {
  return (
    <span className={styles.notice} data-tone={notice.tone}>
      {notice.tone === "working" ? (
        <Spinner size={14} />
      ) : (
        <span aria-hidden="true" className={styles.dot} />
      )}
      <span className={styles.noticeText}>{notice.text}</span>
      {notice.detail && <span className={styles.noticeDetail}>{notice.detail}</span>}
      {notice.fix && <Fix actions={actions} fix={notice.fix} />}
    </span>
  );
}

/**
 * The most important notice, alone; the others behind «+N», each with its
 * own fix. Nothing when all is well.
 */
export function TopBarNotices({
  notices,
  actions,
}: {
  notices: TopBarNotice[];
  actions: NoticeActions;
}) {
  const es = useCopy();
  const [open, setOpen] = useState(false);
  const [first, ...rest] = notices;
  if (!first) return null;
  return (
    <div className={styles.notices}>
      <div aria-live="polite" role="status">
        <NoticeRow actions={actions} notice={first} />
      </div>
      {rest.length > 0 && (
        <Popover
          align="end"
          label={es.problem.all}
          onOpenChange={setOpen}
          open={open}
          trigger={
            <>
              <span aria-hidden="true">{es.problem.more(rest.length)}</span>
              <span className={styles.visuallyHidden}>{es.problem.moreLabel(rest.length)}</span>
            </>
          }
          triggerClassName={styles.more}
        >
          <ul className={styles.rest}>
            {rest.map((notice) => (
              <li key={notice.id}>
                <NoticeRow actions={actions} notice={notice} />
              </li>
            ))}
          </ul>
        </Popover>
      )}
    </div>
  );
}
