import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { configureCopy, getCopy, type Catalogue } from ".";
import { systemLanguages, resolveLanguage, type LanguageChoice } from "./language";

const CopyContext = createContext<{
  copy: Catalogue;
  choice: LanguageChoice;
  languages: readonly string[];
  resolved: ReturnType<typeof resolveLanguage>;
  name: string;
  select: (choice: LanguageChoice) => void;
} | null>(null);

export function CopyProvider({
  name = "Deimos",
  choice: suppliedChoice,
  languages,
  children,
}: {
  name?: string;
  choice?: LanguageChoice;
  languages?: readonly string[];
  children: ReactNode;
}) {
  const parent = useContext(CopyContext);
  const choice = suppliedChoice ?? parent?.choice ?? "system";
  const [preferred, setPreferred] = useState(systemLanguages);
  const [selected, setSelected] = useState({ source: choice, value: choice });
  const shown = selected.source === choice ? selected.value : choice;
  useEffect(() => {
    setSelected((before) =>
      before.source === choice ? before : { source: choice, value: choice },
    );
  }, [choice]);
  useEffect(() => {
    const changed = () => setPreferred(systemLanguages());
    window.addEventListener("languagechange", changed);
    return () => window.removeEventListener("languagechange", changed);
  }, []);
  const supported = languages ?? parent?.languages ?? preferred;
  const copy = configureCopy(shown, supported, name);
  const resolved = resolveLanguage(shown, supported);
  useEffect(() => {
    document.documentElement.lang = resolved.language;
  }, [resolved.language]);
  return (
    <CopyContext
      value={{
        copy,
        choice: shown,
        languages: supported,
        resolved,
        name,
        select: (value) => setSelected({ source: choice, value }),
      }}
    >
      {children}
    </CopyContext>
  );
}

export function useCopy() {
  return useContext(CopyContext)?.copy ?? getCopy();
}

export function useLanguage() {
  const context = useContext(CopyContext);
  return { choice: context?.choice ?? "system", select: context?.select };
}
