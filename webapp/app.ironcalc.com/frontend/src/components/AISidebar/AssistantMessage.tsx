import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

const REMARK_PLUGINS = [remarkGfm];

// Loading an image sends a request without a click, so text planted in a cell
// could get the model to leak sheet data through an image address.
const BLOCKED_ELEMENTS = ["img"];

const COMPONENTS: Components = {
  a: ({ node: _node, children, ...link }) => (
    <a {...link} target="_blank" rel="noopener noreferrer">
      {children}
    </a>
  ),
};

/** Raw HTML in the reply is shown as text, never run: no HTML plugin is enabled. */
export function AssistantMessage(properties: { text: string }) {
  return (
    <div className="app-ic-ai-sidebar-markdown">
      <Markdown
        remarkPlugins={REMARK_PLUGINS}
        components={COMPONENTS}
        disallowedElements={BLOCKED_ELEMENTS}
      >
        {properties.text}
      </Markdown>
    </div>
  );
}
