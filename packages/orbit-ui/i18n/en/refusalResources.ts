const resources: Record<import("../es/refusalResources").RefusalResource, string> = {
  subscriptions: "subscriptions",
  ipc_response: "local response",
  ipc_request: "local request",
  hub_response: "hub response",
  message: "message",
  recording: "recording",
  session: "conversation",
  audio: "audio",
  calendar: "calendar",
  source: "source",
  capture: "capture",
  computer_action: "Mac action",
  storage: "storage",
  folder: "folder",
  file: "file",
};

export function refusalResource(resource: unknown): string {
  const code = String(resource);
  return Object.hasOwn(resources, code)
    ? resources[code as import("../es/refusalResources").RefusalResource]
    : code;
}
