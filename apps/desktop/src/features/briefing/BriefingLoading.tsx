import { useCopy } from "../../i18n/CopyProvider";
import { Skeleton } from "../../design/Skeleton";
import { Loading } from "../../design/Spinner";

import styles from "./BriefingView.module.css";

/**
 * While the agenda is read: the shape of the next meeting and of the day's
 * list, with what is happening beside the spinner. Never blank, never «no
 * meetings» before the agenda says so.
 */
export function BriefingLoading() {
  const es = useCopy();
  return (
    <div aria-busy="true" className={styles.loadingBody}>
      <Loading size={14}>{es.briefing.readingAgenda}</Loading>
      <section className={`${styles.card} ${styles.next}`}>
        <Skeleton height={12} widths={["28%"]} />
        <div className={styles.skeletonGap}>
          <Skeleton height={12} widths={["52%", "86%", "40%"]} />
        </div>
      </section>
      <section className={`${styles.card} ${styles.agenda}`}>
        <Skeleton height={12} widths={["22%"]} />
        <div className={styles.skeletonRows}>
          {["58%", "44%", "66%", "38%"].map((width) => (
            <Skeleton height={11} key={width} widths={[width]} />
          ))}
        </div>
      </section>
    </div>
  );
}
