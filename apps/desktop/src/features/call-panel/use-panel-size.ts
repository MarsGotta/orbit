import { useEffect, useRef } from "react";
import type { DaemonPort } from "../../bridge/port";

/** Measure the intrinsic card plus refusals; the host constrains it to the work area. */
export function usePanelSize(port: DaemonPort) {
  const content = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const element = content.current;
    if (element === null || !port.resizePanel || typeof ResizeObserver === "undefined") return;
    let previous = 0;
    const measure = () => {
      const height = Math.ceil(element.getBoundingClientRect().height);
      if (height > 0 && height !== previous) {
        previous = height;
        void port.resizePanel?.(height);
      }
    };
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    measure();
    return () => observer.disconnect();
  }, [port]);
  return content;
}
