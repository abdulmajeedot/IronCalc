import type { Model } from "@ironcalc/workbook";
import { beforeEach, describe, expect, it, vi } from "vitest";

const storage = vi.hoisted(() => ({
  metadata: {} as Record<string, { name: string }>,
  selected: null as string | null,
  loaded: [] as string[],
  saved: [] as string[],
  created: [] as string[],
}));
const versions = vi.hoisted(() => ({ saved: [] as string[] }));

vi.mock("../storage", () => ({
  getModelsMetadata: () => storage.metadata,
  getSelectedUuid: () => storage.selected,
  loadModelFromStorage: async (uuid: string) => {
    storage.loaded.push(uuid);
    return storage.metadata[uuid]
      ? { name: uuid, toBytes: () => Uint8Array.of(1), free: vi.fn() }
      : null;
  },
  saveModelBytesInStorage: async (uuid: string) => {
    storage.saved.push(uuid);
  },
  createWorkbookInStorage: async (name: string) => {
    storage.created.push(name);
    return "new-uuid";
  },
}));
vi.mock("./versions", () => ({
  saveVersion: async (version: { uuid: string; request: string }) => {
    versions.saved.push(`${version.uuid}: ${version.request}`);
  },
}));

import { createWorkbookDirectory } from "./workbookDirectory";

const active = {
  getName: () => "Open",
  toBytes: () => Uint8Array.of(2),
} as unknown as Model;

function directory(isReadOnly = false) {
  const onActiveChanged = vi.fn();
  const onWorkbookChanged = vi.fn();
  const instance = createWorkbookDirectory({
    getActiveModel: () => active,
    isReadOnly: () => isReadOnly,
    onActiveChanged,
    onWorkbookChanged,
  });
  return { instance, onActiveChanged, onWorkbookChanged };
}

const nameOf = (model: Model) => (model as unknown as { name: string }).name;

describe("workbook directory", () => {
  beforeEach(() => {
    storage.metadata = {
      open: { name: "Open" },
      north: { name: "North Region" },
    };
    storage.selected = "open";
    storage.loaded = [];
    storage.saved = [];
    storage.created = [];
    versions.saved = [];
  });

  it("uses the live model for the open workbook", async () => {
    const { instance, onActiveChanged } = directory();
    instance.beginRequest("do it");
    const used = await instance.run(undefined, "write", (model) => model);
    expect(used).toBe(active);
    expect(storage.loaded).toEqual([]);
    expect(onActiveChanged).toHaveBeenCalledTimes(1);
  });

  it("loads and saves another workbook found by name", async () => {
    const { instance, onWorkbookChanged } = directory();
    instance.beginRequest("do it");
    const used = await instance.run("north region.xlsx", "write", nameOf);
    expect(used).toBe("north");
    expect(storage.saved).toEqual(["north"]);
    expect(onWorkbookChanged).toHaveBeenCalledWith("north");
  });

  it("doesn't save after reading", async () => {
    const { instance } = directory();
    instance.beginRequest("look");
    await instance.run("North Region", "read", nameOf);
    expect(storage.saved).toEqual([]);
    expect(versions.saved).toEqual([]);
  });

  it("keeps one version per workbook per request", async () => {
    const { instance } = directory();
    instance.beginRequest("first");
    await instance.run("North Region", "write", nameOf);
    await instance.run("North Region", "write", nameOf);
    await instance.run(undefined, "write", nameOf);
    instance.beginRequest("second");
    await instance.run("North Region", "write", nameOf);
    expect(versions.saved).toEqual([
      "north: first",
      "open: first",
      "north: second",
    ]);
  });

  it("stays on the requested workbook if the user switches", async () => {
    const { instance } = directory();
    instance.beginRequest("do it");
    storage.selected = "north";
    await expect(instance.run(undefined, "read", nameOf)).resolves.toBe("open");
  });

  it("refuses to change a read-only open workbook", async () => {
    storage.selected = null;
    const { instance } = directory(true);
    instance.beginRequest("do it");
    const work = vi.fn();
    await expect(instance.run(undefined, "write", work)).rejects.toThrow(
      "read-only",
    );
    expect(work).not.toHaveBeenCalled();
  });

  it("can still change saved workbooks from a read-only link", async () => {
    storage.selected = null;
    const { instance } = directory(true);
    instance.beginRequest("do it");
    await instance.run("North Region", "write", nameOf);
    expect(storage.saved).toEqual(["north"]);
  });

  it("explains an unknown workbook", async () => {
    const { instance } = directory();
    await expect(instance.run("Nope", "read", nameOf)).rejects.toThrow(
      "The workbooks are: Open, North Region",
    );
  });

  it("refuses an ambiguous name", async () => {
    storage.metadata.twin = { name: "north region" };
    const { instance } = directory();
    await expect(instance.run("North Region", "read", nameOf)).rejects.toThrow(
      "More than one",
    );
  });

  it("creates a workbook and rejects a duplicate", async () => {
    const { instance, onWorkbookChanged } = directory();
    await expect(instance.create("Summary.xlsx")).resolves.toBe("Summary");
    expect(onWorkbookChanged).toHaveBeenCalledWith("new-uuid");
    await expect(instance.create("north region")).rejects.toThrow(
      "already exists",
    );
  });

  it("lists workbooks and which one is open", () => {
    expect(directory().instance.list()).toEqual([
      { name: "Open", isOpen: true },
      { name: "North Region", isOpen: false },
    ]);
  });
});
