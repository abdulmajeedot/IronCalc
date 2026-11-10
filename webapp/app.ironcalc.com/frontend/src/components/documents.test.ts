import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@ironcalc/workbook", () => ({ Model: { fromBytes: vi.fn() } }));
vi.mock("./storage", () => ({
  getLanguageFromLocale: () => "en",
  getModelsMetadata: () => ({ "local-1": { name: "Book" } }),
  getSelectedUuid: () => "local-1",
  loadDefaultLocaleFromStorage: () => "en-US",
  loadModelFromStorage: vi.fn(),
  saveModelBytesInStorage: vi.fn(),
  saveModelToStorage: vi.fn(),
  selectModelFromStorage: vi.fn(),
}));

import type { Model } from "@ironcalc/workbook";
import {
  InvalidTokenError,
  isSelectedOnBackend,
  isSelectedPaused,
  listDocuments,
  markSelectedDirty,
  sendEdits,
  setToken,
  uploadSelected,
} from "./documents";

const store = new Map<string, string>();
const model = {
  getName: () => "Book",
  toBytes: () => new Uint8Array([1, 2, 3]),
} as unknown as Model;

function answer(status: number, body: unknown = {}) {
  return new Response(JSON.stringify(body), { status });
}

function link(version = 1) {
  fetchMock.mockResolvedValueOnce(
    answer(201, { id: "doc-1", name: "Book", version, updated_at: 0 }),
  );
  return uploadSelected(model);
}

const fetchMock = vi.fn();

beforeEach(() => {
  store.clear();
  fetchMock.mockReset();
  vi.stubGlobal("fetch", fetchMock);
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => store.set(key, value),
    removeItem: (key: string) => store.delete(key),
  });
  setToken("secret");
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("documents", () => {
  it("sends the token", async () => {
    fetchMock.mockResolvedValueOnce(answer(200, []));
    await listDocuments();
    const [, init] = fetchMock.mock.calls[0];
    expect(init.headers.Authorization).toBe("Bearer secret");
  });

  it("treats a refused token differently from other failures", async () => {
    fetchMock.mockResolvedValueOnce(answer(401));
    await expect(listDocuments()).rejects.toBeInstanceOf(InvalidTokenError);
  });

  it("only sends workbooks with edits", async () => {
    await link();
    fetchMock.mockClear();
    await sendEdits(model);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("sends edits with the version it last saw and keeps the new one", async () => {
    await link(3);
    markSelectedDirty();
    fetchMock.mockResolvedValueOnce(answer(200, { version: 4 }));
    await sendEdits(model);
    expect(fetchMock.mock.calls[1][0]).toContain("version=3");

    markSelectedDirty();
    fetchMock.mockResolvedValueOnce(answer(200, { version: 5 }));
    await sendEdits(model);
    expect(fetchMock.mock.calls[2][0]).toContain("version=4");
  });

  it("stops sending when the backend has moved on", async () => {
    await link();
    markSelectedDirty();
    fetchMock.mockResolvedValueOnce(answer(409, { current_version: 9 }));
    const conflict = await sendEdits(model);
    expect(conflict).toEqual({ uuid: "local-1", name: "Book" });
    expect(isSelectedPaused()).toBe(true);

    fetchMock.mockClear();
    markSelectedDirty();
    expect(await sendEdits(model)).toBeNull();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("tries again after a network failure", async () => {
    await link();
    markSelectedDirty();
    fetchMock.mockRejectedValueOnce(new Error("offline"));
    await sendEdits(model);

    fetchMock.mockResolvedValueOnce(answer(200, { version: 2 }));
    await sendEdits(model);
    expect(fetchMock).toHaveBeenCalledTimes(3);
  });

  it("forgets the link when the document was deleted", async () => {
    await link();
    expect(isSelectedOnBackend()).toBe(true);
    markSelectedDirty();
    fetchMock.mockResolvedValueOnce(answer(404));
    await sendEdits(model);
    expect(isSelectedOnBackend()).toBe(false);
  });
});
