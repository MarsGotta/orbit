import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useCopy } from "../../i18n/CopyProvider";

/** Review text without loading images or following links before it is saved. */
export function DiffMarkdown({ text }: { text: string }) {
  const copy = useCopy().vaultReading;
  return (
    <ReactMarkdown
      skipHtml
      remarkPlugins={[remarkGfm]}
      components={{
        a: ({ children }) => <span>{children}</span>,
        img: ({ alt }) => <span>{alt}</span>,
        p: ({ children }) => (
          <>
            {children}
            {"\n"}
          </>
        ),
        input: ({ checked }) => (
          <input
            aria-label={copy.readOnlyTask}
            checked={checked}
            disabled
            readOnly
            type="checkbox"
          />
        ),
      }}
    >
      {text}
    </ReactMarkdown>
  );
}
