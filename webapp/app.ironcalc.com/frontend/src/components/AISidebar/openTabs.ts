const STORAGE_KEY = "ironcalc-ai-open-tabs";

function load(): string[] {
  try {
    const value: unknown = JSON.parse(
      localStorage.getItem(STORAGE_KEY) ?? "[]",
    );
    return Array.isArray(value)
      ? value.filter((item): item is string => typeof item === "string")
      : [];
  } catch {
    return [];
  }
}

// Replaced on every change so React can compare snapshots by reference.
let tabs = load();
const listeners = new Set<() => void>();

function update(next: string[]) {
  tabs = next;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    // The tabs still work until the page is reloaded.
  }
  for (const listener of listeners) {
    listener();
  }
}

export function getOpenTabs(): string[] {
  return tabs;
}

export function subscribeToTabs(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function openWorkbookTab(uuid: string) {
  if (!tabs.includes(uuid)) {
    update([...tabs, uuid]);
  }
}

export function closeWorkbookTab(uuid: string) {
  update(tabs.filter((item) => item !== uuid));
}
