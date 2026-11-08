import { vi } from "vitest";
import type { Distillation } from "orbit-ui/bridge/generated/Distillation";
import type { VaultPort } from "./api";
import { fixturePort } from "./test-port";
export function proposal(): Distillation {
  return {
    id: "cedar-proposal",
    meeting_id: "cedar-meeting",
    source_kind: "meeting",
    title: "Cedar review",
    created_at: "2026-10-30T11:52:00Z",
    by: "orbit-hub/0.14.0",
    revision: 0,
    status: "proposed",
    summary: "Cedar plan",
    can_prepare_again: true,
    error: null,
    changed_paths: [],
    skipped: [],
    dropped: [],
    people: {},
    new_source: { path: "Sources/cedar-review.md", summary: "Cedar plan", sha256: "invented" },
    quotes: [
      { id: "q1", at_ms: 65000, speaker: "Nora Vale", text: "The new plan starts in November." },
    ],
    conflicts: [
      {
        id: "date",
        path: "Projects/cedar-pilot.md",
        property: "kickoff",
        current: "2026-10-26",
        incoming: "2026-11-02",
        quote_ids: ["q1"],
        choice: null,
      },
    ],
    changes: [
      {
        id: "project",
        path: "Projects/cedar-pilot.md",
        kind: "update",
        title: "Cedar Pilot",
        include: true,
        base_hash: "invented",
        properties: [
          { key: "kickoff", before: "2026-10-26", after: "2026-11-02", quote_ids: ["q1"] },
        ],
        lines: [{ section: "Plan", before: "Old plan", after: "New plan", quote_ids: ["q1"] }],
        warnings: [],
      },
      {
        id: "person",
        path: "People/eli-brook.md",
        kind: "create",
        title: "Eli Brook",
        include: true,
        base_hash: null,
        properties: [],
        lines: [{ section: "About", before: "", after: "Coordinates Cedar", quote_ids: ["q1"] }],
        warnings: [],
      },
    ],
  };
}
export function proposalPort(initial = proposal()) {
  let current = structuredClone(initial);
  const base = fixturePort();
  return vi.fn<VaultPort>(async (call) => {
    if (call.call === "list_distillations")
      return { outcome: "distillations", page: { entries: [current], next_page: null } };
    if (call.call === "distillation") return { outcome: "distillation", proposal: current };
    if (call.call === "review_distillation") {
      const value = structuredClone(current);
      value.revision++;
      for (const conflict of value.conflicts)
        conflict.choice = call.review.decisions?.[conflict.id] ?? conflict.choice;
      for (const change of value.changes) {
        const edit = call.review.changes?.[change.id];
        if (edit === undefined) continue;
        change.include = edit.include ?? change.include;
        for (const prop of change.properties)
          prop.after = edit.properties?.[prop.key] ?? prop.after;
        for (const [i, line] of change.lines.entries())
          line.after = edit.lines?.[String(i)] ?? line.after;
      }
      current = value;
      return { outcome: "distillation", proposal: current };
    }
    if (call.call === "distillation_action") {
      current = {
        ...current,
        status:
          call.action === "apply" ? "applied" : call.action === "undo" ? "undone" : "discarded",
      };
      return { outcome: "distillation", proposal: current };
    }
    if (call.call === "prepare_distillation") {
      current = { ...current, status: "queued" };
      return { outcome: "distillation", proposal: current };
    }
    return base(call);
  });
}
