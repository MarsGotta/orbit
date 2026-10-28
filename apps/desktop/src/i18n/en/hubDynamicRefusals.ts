import { formatQuantity } from "../locale";
import type { Numbers } from "./refusalNumbers";

const messages: Record<import("../es/hubDynamicRefusals").HubDynamicCode, string> = {
  agent_adapter_missing:
    "The assistant is unavailable on your hub; publish the hub again to install it.",
  agent_folder_invalid: "The agent folder is not a directory.",
  agent_folder_not_removed:
    "The assistant folder could not be deleted on the hub; it remains listed.",
  agent_mismatch: "This conversation belongs to another assistant; open a new conversation.",
  agent_not_found: "This hub does not have that agent.",
  agent_not_removable:
    "This assistant comes from your hub configuration; only whoever runs it can remove it.",
  agent_primary_required:
    "This is the primary assistant; choose another primary assistant to change it.",
  agent_process_limit: "There is no room for another assistant process.",
  agent_profile_invalid: "Check the assistant name, description and colour.",
  agent_start_timeout: "The assistant did not finish starting; the message was not sent.",
  agent_timeout: "The assistant did not finish within the deadline.",
  approval_choice_mismatch: "The selected choice contradicts the approval; only “deny” rejects.",
  approval_not_allowed: "The agent does not offer that choice in this request.",
  assistant_admin_invalid: "Hermes returned something the hub cannot read.",
  assistant_admin_refused: "Hermes refused the change.",
  assistant_admin_timeout: "Hermes did not respond within the deadline.",
  assistant_admin_too_large: "The Hermes response exceeds the limit.",
  assistant_admin_unreachable: "Could not connect to Hermes for the change.",
  assistant_connect_timeout:
    "The assistant provider did not accept the connection within the deadline.",
  assistant_connection_lost:
    "The assistant provider connection was cut before finishing the response.",
  assistant_context_exceeded: "The model does not support that much context.",
  assistant_http_error: "The assistant provider refused the request.",
  assistant_provider_error: "The assistant provider failed without giving a cause.",
  assistant_rate_limited: "The assistant provider rate limited requests.",
  assistant_timeout: "The assistant provider stopped responding.",
  assistant_tool_failed:
    "The assistant tool failed; its outcome is uncertain and it will not be repeated.",
  assistant_unauthorized: "The assistant provider rejected the key.",
  assistant_unreachable: "Could not connect to the assistant provider.",
  attachments_too_large:
    "The attachments exceed the limit; attach fewer files or open a new conversation.",
  invalid_turn_transition: "Turn transition not allowed.",
  note_revision_changed: "The note changed since you started editing it.",
  quota_exceeded:
    "Server quota reached; raise storage_quota_bytes or free space. The Mac retains its copy.",
  session_not_found:
    "The session is not connected to the assistant; another message will attempt to resume if the previous turn is closed.",
  session_resume_connection_replaced:
    "The assistant connection changed while loading history; the message was not sent.",
  session_resume_history_timeout:
    "The assistant could not read stored history; the message was not sent. Send a new message to resume again.",
  session_resume_in_progress: "The assistant is still loading this conversation’s history; wait.",
  session_resume_progress_unavailable:
    "The assistant did not confirm whether history finished loading; the message was not sent.",
  session_resume_timeout:
    "The assistant did not confirm loading history; the message was not sent.",
  sign_in_not_found: "There is no sign-in for that assistant.",
  sign_in_unsupported:
    "This assistant does not sign in through Orbit; whoever runs your hub configures it.",
  sign_out_failed: "The provider tool did not sign out.",
  sign_out_timeout: "The provider tool did not sign out within the deadline.",
  skill_invalid: "The skill does not meet the SKILL.md standard.",
  speech_http_error: "The speech service returned an HTTP error.",
  speech_not_audio: "The speech service did not return audio.",
  speech_timeout: "The speech service did not respond within the deadline.",
  speech_unreachable: "Could not connect to the speech service.",
  turn_outcome_unknown: "Could not check the turn; interrupt or open a new session.",
  upstream_rejected: "Hermes refused the operation.",
  agent_protocol: "The agent sent a response outside the contract.",
};

export function dynamicHubRefusal(code: string, numbers: Numbers): string | undefined {
  if (!Object.hasOwn(messages, code)) return undefined;
  const facts = Object.entries(numbers)
    .filter(([key]) => !["cause", "detail", "error", "message", "reason"].includes(key))
    .map(([key, value]) => `${key}: ${formatQuantity(value)}`)
    .join(", ");
  return `${messages[code as import("../es/hubDynamicRefusals").HubDynamicCode]}${facts ? ` (${facts})` : ""}`;
}
