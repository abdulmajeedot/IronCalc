// Formulas can name another workbook on the backend, as in
// ='[Sales]Sheet1'!A1. This finds those documents and loads their values.
import type { Model } from "@ironcalc/workbook";
import {
  type BackendDocument,
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

// Loads the values of linked documents that changed. With `onlyNew`, just
// looks for names that haven't been linked yet, for a formula just typed.
export async function refreshLinkedWorkbooks(
  model: Model,
  onlyNew: boolean,
): Promise<LinkRefresh> {
  const result: LinkRefresh = { changed: false, missing: [] };
  if (refreshing || !getToken()) {
    return result;
  }
  const now = Date.now();
  const books = model
    .getExternalBooks()
    .filter(
      (book) =>
        !onlyNew ||
        (!book.document_id &&
          now - (lastLookedFor.get(book.name) ?? 0) >= RETRY_MISSING_MS),
    );
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
