import type { Distillation } from "orbit-ui/bridge/generated/Distillation";
import { quoteTime } from "./distillation-format";
import styles from "./Distillation.module.css";
export function DistillationQuotes({
  proposal,
  ids,
  compact = false,
}: {
  proposal: Distillation;
  ids: string[];
  compact?: boolean;
}) {
  return (
    <div className={compact ? styles.quoteChips : styles.quotes}>
      {proposal.quotes
        .filter((quote) => ids.includes(quote.id))
        .map((quote) =>
          compact ? (
            <span key={quote.id} title={quote.text}>
              {quoteTime(quote.at_ms)} · {quote.speaker}
            </span>
          ) : (
            <blockquote key={quote.id}>
              <small>
                {quoteTime(quote.at_ms)} · {quote.speaker}
              </small>
              <p>{quote.text}</p>
            </blockquote>
          ),
        )}
    </div>
  );
}
