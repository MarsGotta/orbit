import { useEffect, useState } from "react";
import type { VaultOutcome } from "../../bridge/generated/VaultOutcome";
import type { VaultRead } from "../../bridge/generated/VaultRead";
import type { VaultSettings } from "../../bridge/generated/VaultSettings";
import { message, request, type VaultPort } from "./api";

export function useVaultRead(port: VaultPort | undefined, filter: VaultRead | null) {
  const [answer, setAnswer] = useState<VaultOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, retry] = useState(0);
  const key = JSON.stringify(filter);
  useEffect(() => {
    let alive = true;
    setAnswer(null);
    setError(null);
    const parsed = JSON.parse(key) as VaultRead | null;
    if (parsed === null) return;
    void request(port, { call: "read", request: parsed })
      .then((value) => {
        if (alive) setAnswer(value);
      })
      .catch((error: unknown) => {
        if (alive) setError(message(error));
      });
    return () => {
      alive = false;
    };
  }, [port, key, attempt]);
  return { answer, error, retry: () => retry(attempt + 1) };
}

export function useVaultConfiguration(port: VaultPort | undefined) {
  const [configuration, setConfiguration] = useState<VaultSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, retry] = useState(0);
  useEffect(() => {
    let alive = true;
    setError(null);
    void request(port, { call: "config" })
      .then((answer) => {
        if (alive && answer.outcome === "config") setConfiguration(answer.configuration);
      })
      .catch((error: unknown) => {
        if (alive) setError(message(error));
      });
    return () => {
      alive = false;
    };
  }, [port, attempt]);
  return { configuration, setConfiguration, error, retry: () => retry(attempt + 1) };
}
