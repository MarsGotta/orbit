import Link from "@tiptap/extension-link";
import { TableKit } from "@tiptap/extension-table";
import TaskItem from "@tiptap/extension-task-item";
import TaskList from "@tiptap/extension-task-list";
import { Markdown } from "@tiptap/markdown";
import StarterKit from "@tiptap/starter-kit";
import { safeHref } from "../notes/markdown";

/**
 * The notes' editor (Tiptap) with all the Markdown an answer or an agent's
 * personality can carry: code blocks, quotes, rules, tables and task lists.
 * Links are only web links, and are not followed from inside the editor.
 */
export function markdownExtensions() {
  return [
    StarterKit.configure({ link: false, underline: false }),
    Link.configure({
      autolink: false,
      isAllowedUri: (url) => safeHref(url) !== null,
      openOnClick: false,
    }),
    TableKit,
    TaskList,
    TaskItem.configure({ nested: true }),
    Markdown,
  ];
}
