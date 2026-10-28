import { numberFormat } from "../../locale";
import { ATTACHMENT_NOT_FOUND, fileSize, sessionCeilingText, tooManyText } from "../attachments";
import type { Entries, NoticeData } from "./types";

type Value = NoticeData[keyof NoticeData];

const decimals = numberFormat({ maximumFractionDigits: 1 });

/** A notice's number, which the hub sends as a decimal string. */
function amount(value: Value): number | undefined {
  const parsed = typeof value === "string" ? Number(value) : value;
  return typeof parsed === "number" && Number.isFinite(parsed) ? parsed : undefined;
}

/** A number as the person reads it: «40.000». */
function figure(value: Value): string {
  const parsed = amount(value);
  return parsed === undefined ? "?" : decimals.format(parsed);
}

/** A code as it was sent, without grouping: «-32603». */
function code(value: Value): string {
  const parsed = amount(value);
  return parsed === undefined ? "?" : String(parsed);
}

/** Milliseconds as seconds: «10 s», «2,5 s». */
function seconds(value: Value): string {
  const parsed = amount(value);
  return parsed === undefined ? "an unknown duration" : `${decimals.format(parsed / 1000)} s`;
}

/** « (HTTP 503)», or nothing when the hub did not say. */
function http(value: Value): string {
  const parsed = amount(value);
  return parsed === undefined ? "" : ` (HTTP ${numberFormat().format(parsed)})`;
}

const ASK_ADMIN = "Try again. If it happens again, tell whoever manages your hub.";
const NARROWER = "Ask for something more specific or split it into steps.";

/** A turn the hub refused for its files, in the words of the rule that fired. */
function attachmentFailure(data: NoticeData): string | undefined {
  switch (data.code) {
    case "attachments_too_large":
      return sessionCeilingText(amount(data.stored), amount(data.bytes), amount(data.max_bytes));
    case "attachments_too_many":
      return tooManyText(amount(data.count) ?? "?", amount(data.max) ?? "?");
    case "attachment_not_found":
      return ATTACHMENT_NOT_FOUND;
    case "attachment_refused":
      return "The agent rejected an image in this message. Remove it or try another.";
    case "attachment_unreadable":
      return "The hub couldn’t prepare an attachment for the agent. Remove it or attach it again.";
    default:
      return undefined;
  }
}

/**
 * A turn the generic agent's provider ended without an answer
 * (hub-v1: `assistant_*`), with its numbers and what to do.
 */
function providerFailure(data: NoticeData, name: string): string | undefined {
  const provider = `${name}’s provider`;
  switch (data.code) {
    case "assistant_unreachable":
      return `Couldn’t connect to ${name}’s provider within ${seconds(data.timeout_ms)}. Check that the model server is running and reachable from the hub.`;
    case "assistant_connect_timeout":
      return `${provider} didn’t accept the connection within ${seconds(data.timeout_ms)}. Try again; if it happens again, check the model server.`;
    case "assistant_timeout":
      return `${provider} stopped responding for ${seconds(data.timeout_ms)}. Ask again; if it happens again, check the model server.`;
    case "assistant_connection_lost":
      return `The connection to ${name}’s provider dropped before the answer finished. Ask again.`;
    case "assistant_unauthorized":
      return `${provider} rejected the key${http(data.provider_status)}. Whoever manages your hub needs to check the provider key (api_key_file).`;
    case "assistant_http_error":
      return `${provider} returned an error${http(data.provider_status)}. ${ASK_ADMIN}`;
    case "assistant_rate_limited": {
      const limited = `${provider} is rate-limiting requests${http(data.provider_status)}`;
      return amount(data.retry_after_ms) === undefined
        ? `${limited} without saying how long to wait. Ask again later.`
        : `${limited} and asks you to wait ${seconds(data.retry_after_ms)}. Ask again after that.`;
    }
    case "assistant_context_exceeded": {
      const fits = `The conversation won’t fit in ${name}’s model`;
      return amount(data.tokens) === undefined
        ? `${fits} (${figure(data.history_chars)} characters of history). Start a new conversation.`
        : `${fits}: ${figure(data.tokens)} tokens out of a maximum of ${figure(data.max_tokens)}. Start a new conversation.`;
    }
    case "assistant_tool_failed":
      // Only a tool that never answered carries its deadline; what it did is unknown.
      return amount(data.timeout_ms) === undefined
        ? `${name} couldn’t use an Orbit tool and stopped. Ask again; if it happens again, tell whoever manages your hub.`
        : `An Orbit tool didn’t respond within ${seconds(data.timeout_ms)} and ${name} stopped. It’s unclear whether the action happened. Check before repeating it.`;
    case "assistant_turn_too_long":
      return `${name} made ${figure(data.max_requests)} model requests without finishing and stopped. ${NARROWER}`;
    case "assistant_provider_error":
      return `${provider} failed without giving a reason. Try again.`;
    case "assistant_failed":
      return `${name} failed before finishing the answer. Ask again.`;
    default:
      return undefined;
  }
}

/** Why a Claude Code or Codex agent left its turn unfinished. */
function unfinished(stopReason: Value, name: string): string {
  switch (stopReason) {
    case "max_tokens":
      return `${name} reached its response token limit before finishing. Ask for a shorter answer or split the question.`;
    case "max_turn_requests":
    case "error_max_turns":
      return `${name} reached its model request limit for this turn before finishing. ${NARROWER}`;
    case "refusal":
      return `${name} refused to continue this turn.`;
    case "error_max_budget_usd":
      return `${name} reached the configured budget limit for this turn.`;
    case "error_max_structured_output_retries":
      return `${name} couldn’t produce the requested response format.`;
    default:
      return `${name} stopped before finishing the turn.`;
  }
}

function rejected(data: NoticeData, name: string): string {
  const causes: Record<string, string> = {
    billing_error: "couldn’t use account billing",
    invalid_request: "rejected the request format",
    server_error: "reported a Claude service failure",
    model_capacity: "has no available capacity for that model",
    contextWindowExceeded: "ran out of context space",
    rateLimitExceeded: "received too many requests",
    sessionBudgetExceeded: "exhausted the session budget",
    sandboxError: "couldn’t run within its sandbox",
    flexUnavailable: "has no available Flex capacity",
    serverOverloaded: "reported that the service is overloaded",
    cyberPolicy: "rejected the turn under its cybersecurity policy",
    misalignmentPolicyViolation: "rejected the turn under its safety policy",
    tooManyDenials: "stopped after too many denials",
    internalServerError: "reported an internal Codex failure",
    badRequest: "rejected the request format",
    threadRollbackFailed: "couldn’t restore the thread state",
  };
  const reason = typeof data.stop_reason === "string" ? causes[data.stop_reason] : undefined;
  const rpcCode = data.rpc_code ?? data.acp_code;
  const detail = rpcCode === undefined ? "" : ` (code ${code(rpcCode)})`;
  const status = data.provider_status === undefined ? "" : ` (HTTP ${code(data.provider_status)})`;
  return `${name} ${reason ?? "rejected the turn"}${detail}${status}. ${ASK_ADMIN}`;
}

/**
 * A turn a Claude Code or Codex agent (hub-v1: `agent_*`) or Hermes
 * (`upstream_turn_failed`) ended without an answer.
 */
function agentFailure(data: NoticeData, name: string): string | undefined {
  switch (data.code) {
    case "agent_auth_required":
      return `${name} isn’t signed in on the hub, or its session expired. Sign in again in its settings; if your hub doesn’t allow it, ask whoever manages it.`;
    case "agent_rejected":
      return rejected(data, name);
    case "agent_stopped": {
      if (amount(data.exit_status) !== undefined)
        return `${name} stopped during the turn (exit code ${code(data.exit_status)}). Ask again.`;
      const ceiling = amount(data.max_bytes);
      if (ceiling !== undefined)
        return `${name} sent a message larger than the ${fileSize(ceiling)} limit and stopped. Ask again.`;
      return `${name} stopped during the turn. Ask again.`;
    }
    case "agent_protocol":
      return `${name} returned a response the hub couldn’t understand and stopped. Ask again; if it happens again, tell whoever manages your hub.`;
    case "agent_turn_unfinished":
      return unfinished(data.stop_reason, name);
    case "upstream_turn_failed":
      return `${name} ended the turn without a final answer. Ask again if needed.`;
    default:
      return undefined;
  }
}

/**
 * A failed turn in the words of its cause, naming the conversation's agent;
 * a cause without words of its own stays the generic failure with its code.
 */
export function assistantFailed(assistantName: string): Entries<"assistant_failed"> {
  return {
    assistant_failed: {
      class: "failed",
      text: (data) =>
        attachmentFailure(data) ??
        providerFailure(data, assistantName) ??
        agentFailure(data, assistantName),
    },
  };
}
