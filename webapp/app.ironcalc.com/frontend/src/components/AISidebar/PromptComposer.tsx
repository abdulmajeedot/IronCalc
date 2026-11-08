import { Button, type Model, Tooltip } from "@ironcalc/workbook";
import { FileSpreadsheet, Plus, Quote } from "lucide-react";
import {
  type KeyboardEvent,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { useTranslation } from "react-i18next";
import { useModelsMetadata, useSelectedUuid } from "../useStorage";
import { type Citation, formatCitation, readCitation } from "./citations";
import { getOpenTabs, subscribeToTabs } from "./openTabs";
import {
  findReferences,
  formatReference,
  formatWorkbookMention,
} from "./references";
import { selectedRange } from "./workbookTools";

// The widget doesn't announce selection changes, so the bar checks for them.
const SELECTION_POLL_MS = 200;
const MAX_SUGGESTIONS = 6;

interface MentionQuery {
  /** Where the @ is. */
  start: number;
  /** Where the typed name ends. */
  end: number;
  text: string;
}

function findMentionQuery(value: string, caret: number): MentionQuery | null {
  const match = value.slice(0, caret).match(/(?:^|\s)@([^\s@[\]!]*)$/);
  return match
    ? { start: caret - match[1].length - 1, end: caret, text: match[1] }
    : null;
}

function readSelection(model: Model) {
  const sheets = model.getWorksheetsProperties();
  const [sheet, row, column] = model.getSelectedCell();
  return {
    sheet: sheets[model.getSelectedSheet()]?.name ?? "",
    range: selectedRange(model),
    source: readCitation(model, sheet, row, column),
  };
}

function sameCitation(a: Citation | null, b: Citation | null): boolean {
  return (a && formatCitation(a)) === (b && formatCitation(b));
}

/** Shows the prompt with its references highlighted. */
export function PromptText(properties: { text: string }) {
  const { text } = properties;
  const parts: React.ReactNode[] = [];
  let position = 0;
  for (const reference of findReferences(text)) {
    parts.push(text.slice(position, reference.start));
    parts.push(
      <mark key={reference.start} className="app-ic-ai-sidebar-reference">
        {reference.text}
      </mark>,
    );
    position = reference.end;
  }
  parts.push(text.slice(position));
  return <>{parts}</>;
}

export function PromptComposer(properties: {
  model: Model;
  open: boolean;
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  onOpenSource: (citation: Citation) => void;
}) {
  const { model, open, value, onChange, onSubmit, onOpenSource } = properties;
  const { t } = useTranslation();
  const [selection, setSelection] = useState(() => readSelection(model));
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const mirrorRef = useRef<HTMLDivElement>(null);
  const caretRef = useRef<number | null>(null);
  const metadata = useModelsMetadata();
  const selectedUuid = useSelectedUuid();
  const openTabs = useSyncExternalStore(subscribeToTabs, getOpenTabs);
  const [query, setQuery] = useState<MentionQuery | null>(null);
  const [highlighted, setHighlighted] = useState(0);

  const needle = query?.text.toLowerCase();
  const suggestions =
    needle === undefined
      ? []
      : Object.values(metadata)
          .map((item) => item.name)
          .filter((name) => name.toLowerCase().includes(needle))
          .slice(0, MAX_SUGGESTIONS);

  useEffect(() => {
    if (!open) {
      return;
    }
    const update = () => {
      const next = readSelection(model);
      setSelection((previous) =>
        previous.sheet === next.sheet &&
        previous.range === next.range &&
        sameCitation(previous.source, next.source)
          ? previous
          : next,
      );
    };
    update();
    const timer = setInterval(update, SELECTION_POLL_MS);
    return () => clearInterval(timer);
  }, [model, open]);

  const syncMirrorScroll = () => {
    if (mirrorRef.current && textareaRef.current) {
      mirrorRef.current.scrollTop = textareaRef.current.scrollTop;
    }
  };

  // biome-ignore lint/correctness/useExhaustiveDependencies: Runs after each change to the text
  useLayoutEffect(() => {
    if (caretRef.current !== null && textareaRef.current) {
      textareaRef.current.setSelectionRange(caretRef.current, caretRef.current);
      caretRef.current = null;
    }
    syncMirrorScroll();
  }, [value]);

  const insertReference = () => {
    const textarea = textareaRef.current;
    if (!textarea) {
      return;
    }
    const current = readSelection(model);
    const before = value.slice(0, textarea.selectionStart);
    const after = value.slice(textarea.selectionEnd);
    const leading = before && !/\s$/.test(before) ? " " : "";
    const trailing = /^\s/.test(after) ? "" : " ";
    // With several workbooks open, a bare sheet name would be ambiguous.
    const hasSeveralOpen = openTabs.filter((uuid) => metadata[uuid]).length > 1;
    const workbook =
      hasSeveralOpen && selectedUuid ? metadata[selectedUuid]?.name : undefined;
    const inserted = `${leading}${formatReference(current.sheet, current.range, workbook)}${trailing}`;
    caretRef.current = before.length + inserted.length;
    onChange(`${before}${inserted}${after}`);
    textarea.focus();
  };

  const acceptSuggestion = (name: string) => {
    if (!query) {
      return;
    }
    const inserted = `${formatWorkbookMention(name)} `;
    caretRef.current = query.start + inserted.length;
    onChange(value.slice(0, query.start) + inserted + value.slice(query.end));
    setQuery(null);
    textareaRef.current?.focus();
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (suggestions.length > 0) {
      const count = suggestions.length;
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const step = event.key === "ArrowDown" ? 1 : count - 1;
        setHighlighted((index) => (index + step) % count);
        return;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        event.preventDefault();
        acceptSuggestion(suggestions[Math.min(highlighted, count - 1)]);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setQuery(null);
        return;
      }
    }
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      onSubmit();
      return;
    }
    const { selectionStart, selectionEnd } = event.currentTarget;
    if (
      (event.key === "Backspace" || event.key === "Delete") &&
      selectionStart === selectionEnd
    ) {
      // A reference is removed whole, so it can't be left half broken.
      const reference = findReferences(value).find((item) =>
        event.key === "Backspace"
          ? item.end === selectionStart
          : item.start === selectionStart,
      );
      if (reference) {
        event.preventDefault();
        caretRef.current = reference.start;
        onChange(value.slice(0, reference.start) + value.slice(reference.end));
      }
    }
  };

  return (
    <>
      <div className="app-ic-ai-sidebar-context">
        <FileSpreadsheet aria-hidden="true" />
        <span className="app-ic-ai-sidebar-context-name">
          {selection.sheet}
        </span>
        <Tooltip title={t("ai_sidebar.add_reference")}>
          <Button
            variant="outline"
            size="xs"
            startIcon={<Plus />}
            aria-label={t("ai_sidebar.add_reference")}
            // Keeps the cursor where it is in the prompt.
            onMouseDown={(event) => event.preventDefault()}
            onClick={insertReference}
          >
            {selection.range}
          </Button>
        </Tooltip>
      </div>
      {selection.source && (
        <button
          type="button"
          className="app-ic-ai-sidebar-source"
          onClick={() => {
            if (selection.source) {
              onOpenSource(selection.source);
            }
          }}
        >
          <Quote aria-hidden="true" />
          <span>
            {t("ai_sidebar.source", {
              source: `${selection.source.workbook} · ${selection.source.sheet}!${selection.source.cell}`,
            })}
          </span>
        </button>
      )}
      <div className="app-ic-ai-sidebar-editor">
        {suggestions.length > 0 && (
          <div className="app-ic-ai-sidebar-suggestions">
            {suggestions.map((name, index) => (
              <button
                key={name}
                type="button"
                className={`app-ic-ai-sidebar-suggestion${index === Math.min(highlighted, suggestions.length - 1) ? " app-ic-ai-sidebar-suggestion--active" : ""}`}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => acceptSuggestion(name)}
              >
                <FileSpreadsheet aria-hidden="true" />
                <span>{name}</span>
              </button>
            ))}
          </div>
        )}
        <div
          ref={mirrorRef}
          className="app-ic-ai-sidebar-prompt-mirror"
          aria-hidden="true"
        >
          <PromptText text={value} />
          {"\n"}
        </div>
        <textarea
          ref={textareaRef}
          className="app-ic-ai-sidebar-prompt"
          rows={3}
          value={value}
          placeholder={t("ai_sidebar.prompt_placeholder")}
          aria-label={t("ai_sidebar.prompt_placeholder")}
          onChange={(event) => {
            onChange(event.target.value);
            setHighlighted(0);
            setQuery(
              findMentionQuery(event.target.value, event.target.selectionStart),
            );
          }}
          onBlur={() => setQuery(null)}
          onKeyDown={handleKeyDown}
          onScroll={syncMirrorScroll}
        />
      </div>
    </>
  );
}
