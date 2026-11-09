import { formatQuantity } from "../locale";
import { text, type Numbers } from "./refusalNumbers";

export function adapterRefusal(n: Numbers): string {
  const service = text(n, "service");
  if (!service) {
    const raw = text(n, "cause") ?? "adapter_unavailable";
    const key = /^[a-z_][a-z0-9_]*$/.test(raw) ? raw : "adapter_unavailable";
    switch (key) {
      case "missing":
        return "Cannot isolate the assistant: bubblewrap is missing; install it on the hub";
      case "secret_scan_timeout":
        return `Cannot isolate the assistant yet: the hub secret scan did not finish within ${formatQuantity(n.wait_limit_seconds)} seconds. Try again shortly`;
      case "secret_scan_failed":
        return "Cannot isolate the assistant yet: the hub secret scan could not complete. Try again shortly";
      case "secret_scan_not_ready":
        return "Cannot isolate the assistant yet: the hub secret scan has not finished. Try again shortly";
      default:
        return `Cannot create the bubblewrap sandbox (${key}); check user namespaces and the Ubuntu AppArmor profile (bwrap-userns-restrict) to allow bubblewrap`;
    }
  }
  const vendor = service === "claude-code" ? "Claude" : service === "codex" ? "Codex" : service;
  const causes: Record<string, string> = {
    adapter_path_missing: "does not exist",
    adapter_missing: "the executable or its interpreter does not exist on this platform",
    adapter_not_file: "is not a file",
    adapter_permission: "does not have execute permission",
    adapter_format: "the binary cannot be executed on this platform",
    adapter_launch_failed: "the system refused startup",
    adapter_unavailable: "is unavailable",
  };
  const code = text(n, "cause_code") ?? "adapter_unavailable";
  const cause = Object.hasOwn(causes, code) ? causes[code] : `cause ${code}`;
  const path = text(n, "path");
  const detail = path ? `: ${path} (${cause})` : "";
  return `${vendor} isn’t available on your hub${detail}. Redeploy the hub to install it.`;
}
