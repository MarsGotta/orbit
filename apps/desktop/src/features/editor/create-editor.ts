import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { Annotation, Compartment, EditorState, type Extension } from "@codemirror/state";
import { EditorView, keymap, placeholder, type ViewUpdate } from "@codemirror/view";
import { characterLimit } from "./editing";
import { editingKeys, type MenuKeys, menuKeys } from "./editor-keys";
import { livePreview } from "./live-preview";
import { orbitMarkdown } from "./markdown-syntax";
import { suggestionBlock } from "./suggestion";
import { editorTheme } from "./theme";

/** A change that comes from outside the editor (a newer text), not typed. */
export const external = Annotation.define<boolean>();

/** What can change in an editor once it is made. */
export class EditorParts {
  readonly preview = new Compartment();
  readonly label = new Compartment();
  readonly editable = new Compartment();
}

export interface EditorSetup {
  parent: HTMLElement;
  value: string;
  label: string;
  placeholder: string;
  readOnly: boolean;
  maxChars?: number;
  parts: EditorParts;
  menu: () => MenuKeys;
  toggleRaw: () => void;
  refused: () => void;
  settled: (accepted: boolean) => void;
  blurred: () => void;
  updated: (update: ViewUpdate) => void;
}

export function previewOf(raw: boolean): Extension {
  return raw ? EditorView.editorAttributes.of({ class: "cm-md-raw" }) : livePreview;
}

export function labelOf(label: string): Extension {
  return EditorView.contentAttributes.of({ "aria-label": label });
}

export function editableOf(readOnly: boolean): Extension {
  return [EditorState.readOnly.of(readOnly), EditorView.editable.of(!readOnly)];
}

/**
 * The editor: GitHub's Markdown with Orbit's three marks, as typed; the
 * formatted view; its keys; and the ceiling of characters when it has one.
 * Pastes go in as the plain text they are.
 */
export function createEditor(setup: EditorSetup): EditorView {
  const { parts } = setup;
  return new EditorView({
    parent: setup.parent,
    state: EditorState.create({
      doc: setup.value,
      extensions: [
        menuKeys(setup.menu),
        editingKeys(setup.toggleRaw),
        history(),
        keymap.of([...defaultKeymap, ...historyKeymap]),
        markdown({
          base: markdownLanguage,
          extensions: orbitMarkdown,
          addKeymap: false,
          completeHTMLTags: false,
          pasteURLAsLink: false,
        }),
        EditorView.lineWrapping,
        placeholder(setup.placeholder),
        parts.preview.of(previewOf(false)),
        parts.label.of(labelOf(setup.label)),
        parts.editable.of(editableOf(setup.readOnly)),
        setup.maxChars === undefined ? [] : characterLimit(setup.maxChars, setup.refused),
        suggestionBlock(setup.settled),
        editorTheme,
        EditorView.domEventHandlers({
          blur: () => {
            setup.blurred();
            return false;
          },
        }),
        EditorView.updateListener.of(setup.updated),
      ],
    }),
  });
}
