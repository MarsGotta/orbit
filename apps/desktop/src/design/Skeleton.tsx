import styles from "./Skeleton.module.css";

export interface SkeletonProps {
  /** One bar per line, each as wide as what will arrive there («60%»). */
  widths: string[];
  /** 9 to 12 px: the height of the text it stands for. */
  height?: 9 | 10 | 11 | 12;
}

/**
 * «Se rellena»: the shape of what is coming from the hub. Silent to a screen
 * reader; the container that waits says so with `aria-busy`.
 */
export function Skeleton({ widths, height = 12 }: SkeletonProps) {
  return (
    <div aria-hidden="true" className={styles.skeleton}>
      {widths.map((width, index) => (
        <span className={styles.bar} key={index} style={{ width, height }} />
      ))}
    </div>
  );
}
