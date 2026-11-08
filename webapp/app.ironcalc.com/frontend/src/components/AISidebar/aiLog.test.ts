import { beforeEach, describe, expect, it, vi } from "vitest";
import { aiLog, clearAILog, getAILogText } from "./aiLog";

describe("aiLog", () => {
  beforeEach(() => {
    clearAILog();
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "error").mockImplementation(() => {});
  });

  it("records the level, event and detail", () => {
    aiLog("info", "tool_start", { tool: "read_range" });
    expect(getAILogText()).toMatch(/INFO tool_start {"tool":"read_range"}$/);
  });

  it("keeps the message of an Error", () => {
    aiLog("error", "failed", { error: new Error("boom") });
    expect(getAILogText()).toContain('"message":"boom"');
  });

  it("never records an API key", () => {
    aiLog("error", "failed", "bad key sk-or-v1-abc123-def");
    const text = getAILogText();
    expect(text).not.toContain("abc123");
    expect(text).toContain("[key]");
  });

  it("shortens very long details", () => {
    aiLog("info", "big", "x".repeat(10_000));
    expect(getAILogText().length).toBeLessThan(5_000);
  });

  it("keeps only the newest entries", () => {
    for (let index = 0; index < 600; index += 1) {
      aiLog("info", `event-${index}`);
    }
    const text = getAILogText();
    expect(text).not.toContain("event-0\n");
    expect(text).toContain("event-599");
    expect(text.split("\n")).toHaveLength(500);
  });
});
