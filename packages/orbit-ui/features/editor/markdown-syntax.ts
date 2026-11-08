import type { MarkdownConfig } from "@lezer/markdown";

/**
 * The three inline marks Orbit's notes add to GitHub's Markdown, as the
 * Markdown parser's extensions: `==highlight==`, `[[Note title]]` and
 * `@First Last`. They only name ranges of the text; nothing typed is
 * rewritten.
 */

const EQUALS = 61;
const OPEN_BRACKET = 91;
const AT = 64;

const HighlightDelimiter = { resolve: "Highlight", mark: "HighlightMark" };
const SPACE = /\s|^$/;
const PUNCTUATION = /[\p{S}\p{P}]/u;

/** `==text==`, opened and closed the way `~~` is in GitHub's Markdown. */
export const Highlight: MarkdownConfig = {
  defineNodes: ["Highlight", "HighlightMark"],
  parseInline: [
    {
      name: "Highlight",
      parse(cx, next, pos) {
        if (next !== EQUALS || cx.char(pos + 1) !== EQUALS || cx.char(pos + 2) === EQUALS)
          return -1;
        const before = cx.slice(pos - 1, pos);
        const after = cx.slice(pos + 2, pos + 3);
        const spaceBefore = SPACE.test(before);
        const spaceAfter = SPACE.test(after);
        const punctBefore = PUNCTUATION.test(before);
        const punctAfter = PUNCTUATION.test(after);
        return cx.addDelimiter(
          HighlightDelimiter,
          pos,
          pos + 2,
          !spaceAfter && (!punctAfter || spaceBefore || punctBefore),
          !spaceBefore && (!punctBefore || spaceAfter || punctAfter),
        );
      },
      after: "Emphasis",
    },
  ],
};

/** The longest title a link names: a note's title is one line. */
const MAX_LINK_TITLE = 200;

/** `[[Title]]`: a link to another note by its title, on one line. */
export const WikiLink: MarkdownConfig = {
  defineNodes: ["WikiLink", "WikiLinkMark"],
  parseInline: [
    {
      name: "WikiLink",
      parse(cx, next, pos) {
        if (next !== OPEN_BRACKET || cx.char(pos + 1) !== OPEN_BRACKET) return -1;
        const rest = cx.slice(pos + 2, Math.min(cx.end, pos + 2 + MAX_LINK_TITLE + 2));
        const close = rest.indexOf("]]");
        if (close <= 0) return -1;
        const title = rest.slice(0, close);
        if (/[[\]\n]/.test(title)) return -1;
        const end = pos + 2 + close + 2;
        return cx.addElement(
          cx.elt("WikiLink", pos, end, [
            cx.elt("WikiLinkMark", pos, pos + 2),
            cx.elt("WikiLinkMark", end - 2, end),
          ]),
        );
      },
      before: "Link",
    },
  ],
};

/**
 * `@First Last`: a person, by up to three words that start with a
 * capital (a name and two surnames). An `@` inside a word, as in an email
 * address, is not a mention.
 */
export const MENTION = /^@\p{Lu}[\p{L}'’-]*(?: \p{Lu}[\p{L}'’-]*){0,2}/u;

export const Mention: MarkdownConfig = {
  defineNodes: ["Mention"],
  parseInline: [
    {
      name: "Mention",
      parse(cx, next, pos) {
        if (next !== AT) return -1;
        const before = cx.slice(pos - 1, pos);
        if (before !== "" && /[\p{L}\p{N}_.]/u.test(before)) return -1;
        const match = MENTION.exec(cx.slice(pos, Math.min(cx.end, pos + 120)));
        if (match === null) return -1;
        return cx.addElement(cx.elt("Mention", pos, pos + match[0].length));
      },
    },
  ],
};

export const orbitMarkdown = [Highlight, WikiLink, Mention];
