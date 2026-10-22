import { useState } from "react";
import { MarkdownEditor } from "../../features/editor/MarkdownEditor";
import type { Suggestion } from "../../features/editor/suggestion";
import sheet from "./DesignSheet.module.css";

const NOTE =
  "## Para la llamada con Hoteles Ribera\nConfirmar con **Pablo Medina** la fecha de arranque: el *lunes 26*.";
const WRITTEN: Suggestion = {
  author: "Deimos",
  text: "Llevar también la fecha de la formación de Ribera Playa: **martes 27 a las 10:00**.",
};

/**
 * Not drawn in the design: what the principal wrote for a note, marked
 * «Escrito por Deimos» under its line until it is accepted or discarded.
 */
export function EditorSection() {
  const [suggestion, setSuggestion] = useState<Suggestion | null>(WRITTEN);
  return (
    <section className={sheet.section}>
      <h2 className={sheet.heading}>Editor · lo que escribe el asistente</h2>
      <MarkdownEditor
        label="Nota de ejemplo"
        onSuggestionSettled={() => setSuggestion(null)}
        status="Guardado en este Mac · solo lo ves tú"
        suggestion={suggestion}
        value={NOTE}
      />
    </section>
  );
}
