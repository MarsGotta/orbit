import { useState } from "react";
import type { AppCommand } from "../bridge/generated";
import type { UpdateSnapshot } from "../bridge/generated/UpdateSnapshot";
import { Toast } from "../design/Toast";
import { es } from "../i18n/es";

const DISMISSED_VERSION = "orbit.update.dismissed-version";

function dismissedVersion(): string | null {
  try {
    return localStorage.getItem(DISMISSED_VERSION);
  } catch {
    return null;
  }
}

export function UpdateNotice({
  update,
  send,
}: {
  update: UpdateSnapshot;
  send: (command: AppCommand) => unknown;
}) {
  const [dismissed, setDismissed] = useState(dismissedVersion);
  const version = update.version;
  const blocked = Boolean(update.install_blocked);
  if (
    update.state !== "available" ||
    version === null ||
    version.length === 0 ||
    dismissed === version
  )
    return null;
  return (
    <Toast
      key={version}
      kind="done"
      persistent
      message={`${es.settings.updateNotice(version)}${blocked ? ` · ${update.install_blocked}` : ""}`}
      onClose={() => {
        setDismissed(version);
        try {
          localStorage.setItem(DISMISSED_VERSION, version);
        } catch {
          // The dismissal still lasts for this session.
        }
      }}
      action={
        <button
          disabled={blocked}
          onClick={() => send({ command: "update_install" })}
          type="button"
        >
          {es.settings.updateAction}
        </button>
      }
    />
  );
}
