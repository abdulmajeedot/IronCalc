import type { Agent, AgentEvent } from "@earendil-works/pi-agent-core";
import {
  Button,
  IconButton,
  Input,
  type IronCalcHandle,
  type Model,
} from "@ironcalc/workbook";
import { BookOpen, Check, Circle, History, X } from "lucide-react";
import {
  type FormEvent,
  type RefObject,
  useEffect,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { AssistantMessage } from "./AssistantMessage";
import { createWorkbookAgent, FREE_MODEL_ID } from "./agent";
import { aiLog, getAILogText } from "./aiLog";
import type { Citation } from "./citations";
import { openWorkbookTab } from "./openTabs";
import { PromptComposer, PromptText } from "./PromptComposer";
import { VersionsPanel } from "./VersionsPanel";
import {
  createWorkbookDirectory,
  findWorkbookUuid,
  type WorkbookDirectory,
} from "./workbookDirectory";
import {
  describeReferences,
  describeWorkbook,
  type PlanStep,
  selectCitedCell,
} from "./workbookTools";

import "./ai-sidebar.css";

const API_KEY_STORAGE = "ironcalc-ai-openrouter-key";
const MEMORY_STORAGE = "ironcalc-ai-memory";

const TOOL_LABELS: Record<string, string> = {
  read_range: "ai_sidebar.read",
  write_range: "ai_sidebar.wrote",
  format_range: "ai_sidebar.formatted",
  clear_range: "ai_sidebar.cleared",
  pull_cells: "ai_sidebar.pulled",
  list_workbooks: "ai_sidebar.listed",
  create_workbook: "ai_sidebar.created",
};

type EntryKind = "user" | "assistant" | "tool" | "error" | "notice";

interface Entry {
  id: number;
  kind: EntryKind;
  text: string;
}

let nextEntryId = 0;
function entry(kind: EntryKind, text: string): Entry {
  nextEntryId += 1;
  return { id: nextEntryId, kind, text };
}

// Browser storage can throw, for example in a locked-down private window.
function loadText(key: string): string {
  try {
    return localStorage.getItem(key) ?? "";
  } catch {
    return "";
  }
}

function storeText(key: string, value: string) {
  try {
    if (value) {
      localStorage.setItem(key, value);
    } else {
      localStorage.removeItem(key);
    }
  } catch {
    // The value still works until the tab is closed.
  }
}

export function AISidebar(properties: {
  open: boolean;
  model: Model;
  isReadOnly: boolean;
  ironCalcRef: RefObject<IronCalcHandle | null>;
  onClose: () => void;
  onOpenCopy: (model: Model) => void;
  /** Opens a saved workbook and runs `apply` on it before showing it. */
  onOpenWorkbook: (uuid: string, apply: (model: Model) => void) => void;
}) {
  const {
    open,
    model,
    isReadOnly,
    ironCalcRef,
    onClose,
    onOpenCopy,
    onOpenWorkbook,
  } = properties;
  const { t } = useTranslation();
  const [apiKey, setApiKey] = useState(() => loadText(API_KEY_STORAGE));
  const [draftKey, setDraftKey] = useState("");
  const [memory, setMemory] = useState(() => loadText(MEMORY_STORAGE));
  const [view, setView] = useState<"chat" | "versions" | "memory">("chat");
  const [plan, setPlan] = useState<PlanStep[]>([]);
  const directoryRef = useRef<WorkbookDirectory | null>(null);
  const [entries, setEntries] = useState<Entry[]>([]);
  const [prompt, setPrompt] = useState("");
  const [isBusy, setIsBusy] = useState(false);
  const agentRef = useRef<Agent | null>(null);
  const endOfListRef = useRef<HTMLDivElement>(null);

  // A shared workbook can become editable without the model changing.
  const isReadOnlyRef = useRef(isReadOnly);
  isReadOnlyRef.current = isReadOnly;
  const translateRef = useRef(t);
  translateRef.current = t;
  // A conversation can span workbooks, so it outlives switching between them.
  const modelRef = useRef(model);
  modelRef.current = model;

  useEffect(() => {
    setEntries([]);
    setPlan([]);
    setIsBusy(false);
    if (!apiKey) {
      return;
    }
    const directory = createWorkbookDirectory({
      getActiveModel: () => modelRef.current,
      isReadOnly: () => isReadOnlyRef.current,
      onActiveChanged: () => ironCalcRef.current?.workbook?.refresh(),
      onWorkbookChanged: openWorkbookTab,
    });
    directoryRef.current = directory;
    const agent = createWorkbookAgent(directory, apiKey, () =>
      setEntries((list) => [
        ...list,
        entry("notice", translateRef.current("ai_sidebar.step_limit")),
      ]),
    );

    const describeError = (message: string) => {
      const translate = translateRef.current;
      if (message.includes("401")) {
        return translate("ai_sidebar.error_key");
      }
      if (message.includes("429")) {
        return translate("ai_sidebar.error_busy");
      }
      return translate("ai_sidebar.error_other", { message });
    };

    const handleEvent = (event: AgentEvent) => {
      if (event.type === "message_update") {
        const update = event.assistantMessageEvent;
        if (update.type === "text_delta") {
          setEntries((list) => {
            const last = list[list.length - 1];
            return last?.kind === "assistant"
              ? [
                  ...list.slice(0, -1),
                  { ...last, text: last.text + update.delta },
                ]
              : [...list, entry("assistant", update.delta)];
          });
        }
      } else if (
        event.type === "tool_execution_end" &&
        event.toolName === "update_plan" &&
        !event.isError
      ) {
        setPlan(event.result?.details?.plan ?? []);
      } else if (event.type === "tool_execution_end") {
        const translate = translateRef.current;
        const text = event.isError
          ? translate("ai_sidebar.tool_failed", {
              message: event.result?.content?.[0]?.text ?? "",
            })
          : translate(TOOL_LABELS[event.toolName] ?? event.toolName, {
              range: event.result?.details?.range ?? "",
            });
        setEntries((list) => [
          ...list,
          entry(event.isError ? "error" : "tool", text),
        ]);
      } else if (
        event.type === "message_end" &&
        event.message.role === "assistant"
      ) {
        const { stopReason, errorMessage } = event.message;
        // Stopping during a retry wait can surface as an error, not an abort.
        if (stopReason === "aborted" || agent.signal?.aborted) {
          setEntries((list) => [
            ...list,
            entry("notice", translateRef.current("ai_sidebar.stopped")),
          ]);
        } else if (stopReason === "error") {
          setEntries((list) => [
            ...list,
            entry("error", describeError(errorMessage ?? "")),
          ]);
        }
      }
    };

    const unsubscribe = agent.subscribe(handleEvent);
    agentRef.current = agent;
    return () => {
      unsubscribe();
      agent.abort();
      agentRef.current = null;
      directoryRef.current = null;
    };
  }, [apiKey, ironCalcRef]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: Scroll whenever the list changes
  useEffect(() => {
    endOfListRef.current?.scrollIntoView({ block: "end" });
  }, [entries]);

  const saveKey = (event: FormEvent) => {
    event.preventDefault();
    const key = draftKey.trim();
    if (key) {
      storeText(API_KEY_STORAGE, key);
      setApiKey(key);
      setDraftKey("");
    }
  };

  const forgetKey = () => {
    storeText(API_KEY_STORAGE, "");
    setApiKey("");
  };

  const send = async () => {
    const text = prompt.trim();
    const agent = agentRef.current;
    const directory = directoryRef.current;
    if (!text || !agent || !directory || isBusy) {
      return;
    }
    setPrompt("");
    setPlan([]);
    setEntries((list) => [...list, entry("user", text)]);
    setIsBusy(true);
    try {
      directory.beginRequest(text);
      const others = directory
        .list()
        .filter((item) => !item.isOpen)
        .map((item) => item.name);
      const workbook = describeWorkbook(model, others);
      const references = await describeReferences(directory.run, text);
      aiLog("info", "request", { text, workbook, references });
      const instructions = memory.trim()
        ? `The user's saved instructions:\n${memory.trim()}\n\n`
        : "";
      await agent.prompt(
        `${instructions}${workbook}\n\nRequest: ${text}${
          references
            ? `\n\nContents of the referenced cells:\n\n${references}`
            : ""
        }`,
      );
    } catch (e) {
      aiLog("error", "request_failed", { error: e });
      setEntries((list) => [
        ...list,
        entry("error", t("ai_sidebar.error_other", { message: String(e) })),
      ]);
    } finally {
      if (agentRef.current === agent) {
        setIsBusy(false);
      }
    }
  };

  const openSource = (citation: Citation) => {
    const uuid = findWorkbookUuid(citation.workbook);
    if (!uuid) {
      aiLog("warn", "source_not_found", citation);
      setEntries((list) => [
        ...list,
        entry(
          "notice",
          t("ai_sidebar.source_missing", { name: citation.workbook }),
        ),
      ]);
      return;
    }
    onOpenWorkbook(uuid, (target) => {
      if (!selectCitedCell(target, citation)) {
        aiLog("warn", "source_sheet_not_found", citation);
      }
    });
  };

  const copyLog = async () => {
    try {
      await navigator.clipboard.writeText(getAILogText());
      setEntries((list) => [
        ...list,
        entry("notice", t("ai_sidebar.log_copied")),
      ]);
    } catch (e) {
      aiLog("warn", "copy_log_failed", { error: e });
    }
  };

  return (
    <aside
      className="app-ic-ai-sidebar"
      hidden={!open}
      aria-label={t("ai_sidebar.title")}
    >
      <div className="app-ic-ai-sidebar-header">
        <span className="app-ic-ai-sidebar-title">{t("ai_sidebar.title")}</span>
        <div className="app-ic-ai-sidebar-header-actions">
          {apiKey && (
            <>
              <IconButton
                icon={<History />}
                aria-label={t("ai_sidebar.versions")}
                aria-pressed={view === "versions"}
                size="sm"
                onClick={() =>
                  setView(view === "versions" ? "chat" : "versions")
                }
              />
              <IconButton
                icon={<BookOpen />}
                aria-label={t("ai_sidebar.memory")}
                aria-pressed={view === "memory"}
                size="sm"
                onClick={() => setView(view === "memory" ? "chat" : "memory")}
              />
            </>
          )}
          <IconButton
            icon={<X />}
            aria-label={t("ai_sidebar.close")}
            size="sm"
            onClick={onClose}
          />
        </div>
      </div>
      {apiKey && view === "versions" && (
        <VersionsPanel onOpenCopy={onOpenCopy} />
      )}
      {apiKey && view === "memory" && (
        <div className="app-ic-ai-sidebar-panel">
          <div className="app-ic-ai-sidebar-panel-title">
            {t("ai_sidebar.memory")}
          </div>
          <div className="app-ic-ai-sidebar-hint">
            {t("ai_sidebar.memory_help")}
          </div>
          <textarea
            className="app-ic-ai-sidebar-memory"
            value={memory}
            aria-label={t("ai_sidebar.memory")}
            onChange={(event) => {
              setMemory(event.target.value);
              storeText(MEMORY_STORAGE, event.target.value);
            }}
          />
        </div>
      )}
      {apiKey ? (
        <div className="app-ic-ai-sidebar-chat" hidden={view !== "chat"}>
          <div className="app-ic-ai-sidebar-list">
            {plan.length > 0 && (
              <div className="app-ic-ai-sidebar-plan">
                <div className="app-ic-ai-sidebar-panel-title">
                  {t("ai_sidebar.plan")}
                </div>
                {plan.map((step) => (
                  <div
                    key={step.text}
                    className={`app-ic-ai-sidebar-plan-step${step.done ? " app-ic-ai-sidebar-plan-step--done" : ""}`}
                  >
                    {step.done ? <Check /> : <Circle />}
                    <span>{step.text}</span>
                  </div>
                ))}
              </div>
            )}
            {isReadOnly && (
              <div className="app-ic-ai-sidebar-entry app-ic-ai-sidebar-entry--notice">
                {t("ai_sidebar.read_only")}
              </div>
            )}
            {entries.length === 0 && (
              <div className="app-ic-ai-sidebar-entry app-ic-ai-sidebar-entry--notice">
                {t("ai_sidebar.empty")}
              </div>
            )}
            {entries.map((item) => (
              <div
                key={item.id}
                className={`app-ic-ai-sidebar-entry app-ic-ai-sidebar-entry--${item.kind}`}
              >
                {item.kind === "assistant" ? (
                  <AssistantMessage text={item.text} />
                ) : item.kind === "user" ? (
                  <PromptText text={item.text} />
                ) : (
                  item.text
                )}
              </div>
            ))}
            {isBusy && (
              <div className="app-ic-ai-sidebar-entry app-ic-ai-sidebar-entry--notice">
                {t("ai_sidebar.working")}
              </div>
            )}
            <div ref={endOfListRef} />
          </div>
          <div className="app-ic-ai-sidebar-composer">
            <PromptComposer
              model={model}
              open={open}
              value={prompt}
              onChange={setPrompt}
              onSubmit={send}
              onOpenSource={openSource}
            />
            <div className="app-ic-ai-sidebar-actions">
              {isBusy ? (
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => agentRef.current?.abort()}
                >
                  {t("ai_sidebar.stop")}
                </Button>
              ) : (
                <Button size="sm" disabled={!prompt.trim()} onClick={send}>
                  {t("ai_sidebar.send")}
                </Button>
              )}
              {entries.some((item) => item.kind === "error") && (
                <Button variant="ghost" size="sm" onClick={copyLog}>
                  {t("ai_sidebar.copy_log")}
                </Button>
              )}
              <Button variant="ghost" size="sm" onClick={forgetKey}>
                {t("ai_sidebar.forget_key")}
              </Button>
            </div>
            <div className="app-ic-ai-sidebar-hint">
              {FREE_MODEL_ID} · {t("ai_sidebar.undo_hint")}
            </div>
          </div>
        </div>
      ) : (
        <form className="app-ic-ai-sidebar-key-form" onSubmit={saveKey}>
          <Input
            type="password"
            autoComplete="off"
            label={t("ai_sidebar.key_label")}
            helperText={t("ai_sidebar.key_help")}
            value={draftKey}
            onChange={(event) => setDraftKey(event.target.value)}
          />
          <Button type="submit" size="sm" disabled={!draftKey.trim()}>
            {t("ai_sidebar.save_key")}
          </Button>
        </form>
      )}
    </aside>
  );
}
