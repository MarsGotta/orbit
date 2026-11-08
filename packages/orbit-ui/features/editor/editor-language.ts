import { Facet } from "@codemirror/state";
import { getCopy, type Catalogue } from "../../i18n";

/** CodeMirror's widgets follow copy changes without recreating the editor,
 * losing its undo history or moving its selection. */
export const editorLanguage = Facet.define<Catalogue, Catalogue>({
  combine: (values) => values[0] ?? getCopy(),
});
