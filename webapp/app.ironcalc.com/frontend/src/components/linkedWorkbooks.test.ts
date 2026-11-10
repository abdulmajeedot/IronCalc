import { beforeEach, describe, expect, it, vi } from "vitest";

const documents = vi.hoisted(() => ({
  getSelectedDocumentId: vi.fn(),
  getToken: vi.fn(),
  listDocuments: vi.fn(),
  readDocumentSnapshot: vi.fn(),
}));
vi.mock("./documents", () => documents);

import type { Model } from "@ironcalc/workbook";
import { readsBack, refreshLinkedWorkbooks } from "./linkedWorkbooks";

type ExternalBook = ReturnType<Model["getExternalBooks"]>[number];

function fakeModel(books: ExternalBook[]) {
  const setLinkedWorkbook = vi.fn();
  const model = {
    getExternalBooks: () => books,
    setLinkedWorkbook,
  } as unknown as Model;
  return { model, setLinkedWorkbook };
}

const sales = { id: "doc-sales", name: "Sales", version: 3, updated_at: 0 };
const snapshot = {
  version: 3,
  sheets: [{ name: "Sheet1", cells: [[1, 1, 10]] }],
};

let uniqueName = 0;
function freshName() {
  uniqueName += 1;
  return `Book${uniqueName}`;
}

beforeEach(() => {
  vi.resetAllMocks();
  documents.getToken.mockReturnValue("secret");
  documents.getSelectedDocumentId.mockReturnValue(null);
  documents.listDocuments.mockResolvedValue([sales]);
  documents.readDocumentSnapshot.mockResolvedValue(snapshot);
});

describe("refreshLinkedWorkbooks", () => {
  it("finds a document by the name in the formula, ignoring case", async () => {
    const { model, setLinkedWorkbook } = fakeModel([
      { name: "SALES", document_id: null, version: null },
    ]);
    const result = await refreshLinkedWorkbooks(model, false);
    expect(result.changed).toBe(true);
    expect(setLinkedWorkbook).toHaveBeenCalledWith(
      "SALES",
      "doc-sales",
      3,
      snapshot.sheets,
    );
  });

  it("follows the document id after a rename", async () => {
    documents.listDocuments.mockResolvedValue([
      { ...sales, name: "Renamed", version: 4 },
    ]);
    const { model, setLinkedWorkbook } = fakeModel([
      { name: "Sales", document_id: "doc-sales", version: 3 },
    ]);
    await refreshLinkedWorkbooks(model, false);
    expect(setLinkedWorkbook).toHaveBeenCalledOnce();
  });

  it("does nothing when the version is the same", async () => {
    const { model, setLinkedWorkbook } = fakeModel([
      { name: "Sales", document_id: "doc-sales", version: 3 },
    ]);
    const result = await refreshLinkedWorkbooks(model, false);
    expect(result.changed).toBe(false);
    expect(setLinkedWorkbook).not.toHaveBeenCalled();
    expect(documents.readDocumentSnapshot).not.toHaveBeenCalled();
  });

  it("won't link a workbook to itself", async () => {
    documents.getSelectedDocumentId.mockReturnValue("doc-sales");
    const { model, setLinkedWorkbook } = fakeModel([
      { name: "Sales", document_id: null, version: null },
    ]);
    const result = await refreshLinkedWorkbooks(model, false);
    expect(setLinkedWorkbook).not.toHaveBeenCalled();
    expect(result.missing).toEqual(["Sales"]);
  });

  it("keeps the last values and reports a source that is gone", async () => {
    documents.listDocuments.mockResolvedValue([]);
    const { model, setLinkedWorkbook } = fakeModel([
      { name: "Sales", document_id: "doc-sales", version: 3 },
    ]);
    const result = await refreshLinkedWorkbooks(model, false);
    expect(setLinkedWorkbook).not.toHaveBeenCalled();
    expect(result.missing).toEqual(["Sales"]);
  });

  it("doesn't keep asking for a name nothing has", async () => {
    documents.listDocuments.mockResolvedValue([]);
    const name = freshName();
    const { model } = fakeModel([{ name, document_id: null, version: null }]);
    await refreshLinkedWorkbooks(model, true);
    await refreshLinkedWorkbooks(model, true);
    expect(documents.listDocuments).toHaveBeenCalledOnce();
  });

  it("treats a missing document id from the engine as not linked", async () => {
    const name = freshName();
    documents.listDocuments.mockResolvedValue([{ ...sales, name }]);
    const { model, setLinkedWorkbook } = fakeModel([{ name } as ExternalBook]);
    await refreshLinkedWorkbooks(model, true);
    expect(setLinkedWorkbook).toHaveBeenCalledOnce();
  });

  it("updates while open when the source has a new version", async () => {
    const { model, setLinkedWorkbook } = fakeModel([
      { name: "Sales", document_id: "doc-sales", version: 2 },
    ]);
    const result = await refreshLinkedWorkbooks(model, true);
    expect(result.changed).toBe(true);
    expect(setLinkedWorkbook).toHaveBeenCalledOnce();
  });

  it("stops updating while open when two workbooks read each other", async () => {
    documents.getSelectedDocumentId.mockReturnValue("doc-report");
    documents.listDocuments.mockResolvedValue([
      { ...sales, version: 9, sources: ["doc-report"] },
      { ...sales, id: "doc-report", name: "Report", sources: ["doc-sales"] },
    ]);
    const { model, setLinkedWorkbook } = fakeModel([
      { name: "Sales", document_id: "doc-sales", version: 3 },
    ]);
    const live = await refreshLinkedWorkbooks(model, true);
    expect(live.circular).toEqual(["Sales"]);
    expect(setLinkedWorkbook).not.toHaveBeenCalled();

    const onOpening = await refreshLinkedWorkbooks(model, false);
    expect(onOpening.circular).toEqual([]);
    expect(setLinkedWorkbook).toHaveBeenCalledOnce();
  });

  it("does nothing without a token", async () => {
    documents.getToken.mockReturnValue(null);
    const { model } = fakeModel([
      { name: "Sales", document_id: null, version: null },
    ]);
    await refreshLinkedWorkbooks(model, false);
    expect(documents.listDocuments).not.toHaveBeenCalled();
  });
});

describe("readsBack", () => {
  const doc = (id: string, sources: string[]) => ({
    id,
    name: id,
    version: 1,
    updated_at: 0,
    sources,
  });

  it("follows a chain of documents", () => {
    const docs = [doc("a", ["b"]), doc("b", ["c"]), doc("c", ["self"])];
    expect(readsBack(docs, "a", "self")).toBe(true);
  });

  it("is false when nothing reads the workbook", () => {
    const docs = [doc("a", ["b"]), doc("b", [])];
    expect(readsBack(docs, "a", "self")).toBe(false);
  });

  it("copes with loops that don't include the workbook", () => {
    const docs = [doc("a", ["b"]), doc("b", ["a"])];
    expect(readsBack(docs, "a", "self")).toBe(false);
  });

  it("is false for a workbook that isn't on the backend", () => {
    expect(readsBack([doc("a", ["a"])], "a", null)).toBe(false);
  });
});
