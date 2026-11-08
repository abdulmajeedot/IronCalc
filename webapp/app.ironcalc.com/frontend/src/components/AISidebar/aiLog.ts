export type AILogLevel = "info" | "warn" | "error";

export interface AILogEntry {
  time: string;
  level: AILogLevel;
  event: string;
  detail?: string;
}

const MAX_ENTRIES = 500;
const MAX_DETAIL_LENGTH = 4_000;

const entries: AILogEntry[] = [];

function serialize(data: unknown): string {
  try {
    return (
      JSON.stringify(data, (_key, value) =>
        value instanceof Error
          ? { name: value.name, message: value.message, stack: value.stack }
          : value,
      ) ?? String(data)
    );
  } catch {
    return String(data);
  }
}

function clean(data: unknown): string {
  // The key is never logged on purpose. This catches one echoed in an error.
  const text = serialize(data).replace(/sk-or-[\w-]+/g, "[key]");
  return text.length > MAX_DETAIL_LENGTH
    ? `${text.slice(0, MAX_DETAIL_LENGTH)}… (${text.length} characters)`
    : text;
}

/** Writes to the browser console and to the buffer behind "Copy debug log". */
export function aiLog(level: AILogLevel, event: string, data?: unknown) {
  const entry: AILogEntry = {
    time: new Date().toISOString(),
    level,
    event,
    ...(data === undefined ? {} : { detail: clean(data) }),
  };
  entries.push(entry);
  if (entries.length > MAX_ENTRIES) {
    entries.shift();
  }
  console[level]("[ai-sidebar]", event, entry.detail ?? "");
}

export function getAILogText(): string {
  return entries
    .map(
      ({ time, level, event, detail }) =>
        `${time} ${level.toUpperCase()} ${event}${detail ? ` ${detail}` : ""}`,
    )
    .join("\n");
}

export function clearAILog() {
  entries.length = 0;
}
