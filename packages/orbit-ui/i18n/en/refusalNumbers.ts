import type { AgentRefusal } from "../../bridge/generated/AgentRefusal";

export type Numbers = AgentRefusal["numbers"];

export function text(numbers: Numbers, key: string): string | undefined {
  const value = numbers[key];
  return typeof value === "string" ? value : undefined;
}

export function count(numbers: Numbers, key: string): number | undefined {
  const value = numbers[key];
  return typeof value === "number" ? value : undefined;
}
