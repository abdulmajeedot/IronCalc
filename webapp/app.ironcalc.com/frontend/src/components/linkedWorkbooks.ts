// Formulas can name another workbook on the backend, as in
// ='[Sales]Sheet1'!A1. This finds those documents and loads their values.
import type { Model } from "@ironcalc/workbook";
import {
  type BackendDocument,
  type DocumentSnapshot,
  getSelectedDocumentId,
  getToken,
  listDocuments,
  readDocumentSnapshot,
} from "./documents";

// How long to wait before looking again for a name no document has.
const RETRY_MISSING_MS = 30_000;

const lastLookedFor = new Map<string, number>();
let refreshing = false;

export interface LinkRefresh {
  changed: boolean;
  missing: string[];
  // Linked workbooks that read this one back: not updated while it is open
  circular: string[];
}

// Whether a document ends up reading the workbook `selfId`.
export function readsBack(
  documents: BackendDocument[],
  sourceId: string,
  selfId: string | null,
): boolean {
  if (!selfId) {
    return false;
  }
  const seen = new Set<string>();
  const pending = [sourceId];
  while (pending.length > 0) {
    const id = pending.pop() as string;
    if (id === selfId) {
      return true;
    }
    if (!seen.has(id)) {
      seen.add(id);
      pending.push(...(documents.find((d) => d.id === id)?.sources ?? []));
    }
  }
  return false;
}

function findDocument(
  documents: BackendDocument[],
  name: string,
  documentId: string | null | undefined,
  selfId: string | null,
): BackendDocument | undefined {
  if (documentId) {
    return documents.find((document) => document.id === documentId);
  }
  const wanted = name.toUpperCase();
  return documents.find(
    (document) =>
      document.id !== selfId && document.name.toUpperCase() === wanted,
  );
}

// Loads the values of linked documents that changed. `live` is for checks
// made while a workbook stays open: names nothing has are looked for less
// often, and workbooks that read this one back are left alone, or the two
// would keep changing each other.
export async function refreshLinkedWorkbooks(
  model: Model,
  live: boolean,
): Promise<LinkRefresh> {
  const result: LinkRefresh = { changed: false, missing: [], circular: [] };
  if (refreshing || !getToken()) {
    return result;
  }
  const now = Date.now();
  const known = model.getExternalBooks();
  // A name nothing had moments ago still has nothing: no need to ask again.
  const isWaiting = (book: (typeof known)[number]) =>
    live &&
    !book.document_id &&
    now - (lastLookedFor.get(book.name) ?? 0) < RETRY_MISSING_MS;
  result.missing = known.filter(isWaiting).map((book) => book.name);
  const books = known.filter((book) => !isWaiting(book));
  if (books.length === 0) {
    return result;
  }
  refreshing = true;
  try {
    const documents = await listDocuments();
    const selfId = getSelectedDocumentId();
    for (const book of books) {
      const document = findDocument(
        documents,
        book.name,
        book.document_id,
        selfId,
      );
      if (!document) {
        lastLookedFor.set(book.name, now);
        result.missing.push(book.name);
        continue;
      }
      lastLookedFor.delete(book.name);
      if (
        live &&
        book.document_id &&
        readsBack(documents, document.id, selfId)
      ) {
        result.circular.push(book.name);
        continue;
      }
      if (book.version === document.version) {
        continue;
      }
      const snapshot = await readDocumentSnapshot(document.id);
      model.setLinkedWorkbook(
        book.name,
        document.id,
        snapshot.version,
        snapshot.sheets as Parameters<Model["setLinkedWorkbook"]>[3],
      );
      result.changed = true;
    }
  } finally {
    refreshing = false;
  }
  return result;
}

const RANGE = /^\$?[A-Z]{1,3}\$?[1-9]\d*(:\$?[A-Z]{1,3}\$?[1-9]\d*)?$/i;

export function isValidRange(range: string): boolean {
  return RANGE.test(range.trim());
}

export function columnName(column: number): string {
  let name = "";
  for (let rest = column; rest > 0; rest = Math.floor((rest - 1) / 26)) {
    name = String.fromCharCode(65 + ((rest - 1) % 26)) + name;
  }
  return name;
}

// Picked cells as a fixed reference, the way Excel writes one to another file.
export function selectionRange(
  a: { row: number; column: number },
  b: { row: number; column: number },
): string {
  const cell = (row: number, column: number) => `$${columnName(column)}$${row}`;
  const first = cell(Math.min(a.row, b.row), Math.min(a.column, b.column));
  const last = cell(Math.max(a.row, b.row), Math.max(a.column, b.column));
  return first === last ? first : `${first}:${last}`;
}

export function canLinkTo(name: string): boolean {
  return name !== "" && !/[[\]']/.test(name);
}

export function referenceFormula(
  book: string,
  sheet: string,
  range: string,
): string {
  return `='[${book}]${sheet.replace(/'/g, "''")}'!${range.trim().toUpperCase()}`;
}

export type InsertFailure = "invalid_name" | "name_in_use";

// Links a document and puts a reference to it in the selected cell.
export function insertReference(
  model: Model,
  document: BackendDocument,
  snapshot: DocumentSnapshot,
  sheet: string,
  range: string,
): InsertFailure | null {
  if (!canLinkTo(document.name)) {
    return "invalid_name";
  }
  const taken = model
    .getExternalBooks()
    .find((book) => book.name === document.name);
  if (taken?.document_id && taken.document_id !== document.id) {
    return "name_in_use";
  }
  model.setLinkedWorkbook(
    document.name,
    document.id,
    snapshot.version,
    snapshot.sheets as Parameters<Model["setLinkedWorkbook"]>[3],
  );
  const view = model.getSelectedView();
  model.setUserInput(
    view.sheet,
    view.row,
    view.column,
    referenceFormula(document.name, sheet, range),
  );
  return null;
}
