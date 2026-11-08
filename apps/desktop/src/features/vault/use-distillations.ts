import { useEffect, useState, type Dispatch, type SetStateAction } from "react";
import type { Distillation } from "orbit-ui/bridge/generated/Distillation";
import { message, request, type VaultPort } from "./api";
type Page = {
  entries: Distillation[];
  failure: unknown;
  loading: boolean;
  nextPage: number | null;
};
const empty: Page = { entries: [], failure: null, loading: true, nextPage: null };
/** Retain one bounded response page, rather than every proposal's text. */
export function useDistillations(port: VaultPort | undefined, meetingId: string | null = null) {
  const [value, setValue] = useState(empty);
  const [attempt, setAttempt] = useState(0);
  const [page, setPage] = useState(1);
  useEffect(
    () => watchProposals(port, meetingId, page, setValue),
    [port, meetingId, page, attempt],
  );
  return {
    entries: value.entries,
    loading: value.loading,
    page,
    nextPage: value.nextPage,
    setPage,
    error: value.failure === null ? null : message(value.failure),
    reload: () => setAttempt((n) => n + 1),
  };
}
export type DistillationList = ReturnType<typeof useDistillations>;

function watchProposals(
  port: VaultPort | undefined,
  meetingId: string | null,
  page: number,
  setValue: Dispatch<SetStateAction<Page>>,
) {
  let alive = true,
    reading = false;
  let timer: number;
  setValue(empty);
  const read = async () => {
    if (!alive || reading) return;
    if (port === undefined) {
      setValue({ ...empty, loading: false });
      return;
    }
    reading = true;
    try {
      const answer = await request(port, {
        call: "list_distillations",
        meeting_id: meetingId,
        page,
      });
      if (alive && answer.outcome === "distillations")
        setValue({
          entries: answer.page.entries,
          nextPage: answer.page.next_page,
          failure: null,
          loading: false,
        });
    } catch (error: unknown) {
      if (alive) setValue((before) => ({ ...before, failure: error }));
    } finally {
      reading = false;
      if (alive) {
        setValue((before) => ({ ...before, loading: false }));
        window.clearTimeout(timer);
        timer = window.setTimeout(() => void read(), 10000);
      }
    }
  };
  const refresh = () => {
    window.clearTimeout(timer);
    void read();
  };
  void read();
  window.addEventListener("orbit-vault-changed", refresh);
  window.addEventListener("focus", refresh);
  return () => {
    alive = false;
    window.clearTimeout(timer);
    window.removeEventListener("orbit-vault-changed", refresh);
    window.removeEventListener("focus", refresh);
  };
}
