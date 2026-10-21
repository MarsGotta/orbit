import { createContext, useContext, type ReactNode } from "react";
import { es, spanish } from "./es";

const AssistantCopyContext = createContext(es);

export function AssistantCopy({ name, children }: { name?: string; children: ReactNode }) {
  return <AssistantCopyContext value={spanish(name)}>{children}</AssistantCopyContext>;
}

export function useSpanish() {
  return useContext(AssistantCopyContext);
}
