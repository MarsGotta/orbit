/** Fixed hub failures have codes; provider explanations remain conversation content. */
export function toolFailureText(cause: string): string {
  if (cause === "tool_failed") return "La herramienta falló";
  if (cause === "tool_failure_unreported") return "sin causa declarada";
  if (/^[a-z_][a-z0-9_]*$/.test(cause)) return `La herramienta falló (código ${cause}).`;
  return cause;
}
