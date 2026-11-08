/** Fixed hub failures have codes; provider explanations remain conversation content. */
export function toolFailureText(cause: string): string {
  if (cause === "tool_failed") return "The tool failed";
  if (cause === "tool_failure_unreported") return "no cause supplied";
  if (/^[a-z_][a-z0-9_]*$/.test(cause)) return `The tool failed (code ${cause}).`;
  return cause;
}
