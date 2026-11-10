// Workbooks kept on the backend. A workbook on the backend is opened as a
// local copy that remembers which document it is, and edits are sent back.
import { Model } from "@ironcalc/workbook";
import {
  getLanguageFromLocale,
  getModelsMetadata,
  getSelectedUuid,
  loadDefaultLocaleFromStorage,
  loadModelFromStorage,
  saveModelBytesInStorage,
  saveModelToStorage,
  selectModelFromStorage,
} from "./storage";

const TOKEN_KEY = "documents_token";
const LINKS_KEY = "documents_links";

export interface BackendDocument {
  id: string;
  name: string;
  version: number;
  updated_at: number;
  // The documents this one reads values from
  sources?: string[];
}

interface DocumentLink {
  documentId: string;
  version: number;
  // Has edits the backend doesn't have yet
  dirty: boolean;
  // The backend moved on without these edits, so they aren't being sent
  paused: boolean;
}

type Links = Record<string, DocumentLink>;

export class InvalidTokenError extends Error {}

export function getToken(): string | null {
  return localStorage.getItem(TOKEN_KEY);
}

export function setToken(token: string): void {
  localStorage.setItem(TOKEN_KEY, token);
}

export function clearToken(): void {
  localStorage.removeItem(TOKEN_KEY);
}

function readLinks(): Links {
  try {
    return JSON.parse(localStorage.getItem(LINKS_KEY) ?? "{}") as Links;
  } catch {
    return {};
  }
}

function writeLinks(links: Links): void {
  localStorage.setItem(LINKS_KEY, JSON.stringify(links));
}

function updateLink(uuid: string, change: Partial<DocumentLink>): void {
  const links = readLinks();
  if (links[uuid]) {
    links[uuid] = { ...links[uuid], ...change };
    writeLinks(links);
  }
}

function removeLink(uuid: string): void {
  const links = readLinks();
  delete links[uuid];
  writeLinks(links);
}

async function request(path: string, init: RequestInit = {}) {
  const response = await fetch(`/api/documents${path}`, {
    ...init,
    headers: { ...init.headers, Authorization: `Bearer ${getToken()}` },
  });
  if (response.status === 401) {
    throw new InvalidTokenError();
  }
  return response;
}

function failed(response: Response): Error {
  return new Error(`The backend answered ${response.status}`);
}

export async function listDocuments(): Promise<BackendDocument[]> {
  const response = await request("");
  if (!response.ok) {
    throw failed(response);
  }
  return (await response.json()) as BackendDocument[];
}

async function downloadDocument(
  id: string,
): Promise<{ model: Model; version: number }> {
  const response = await request(`/${encodeURIComponent(id)}`);
  if (!response.ok) {
    throw failed(response);
  }
  const bytes = new Uint8Array(await response.arrayBuffer());
  const language = getLanguageFromLocale(loadDefaultLocaleFromStorage());
  return {
    model: Model.fromBytes(bytes, language),
    version: Number(response.headers.get("X-Document-Version")),
  };
}

// The local workbook that is this document, if it is still around.
function localCopyOf(documentId: string): string | null {
  const metadata = getModelsMetadata();
  const links = readLinks();
  return (
    Object.keys(links).find(
      (uuid) => links[uuid].documentId === documentId && metadata[uuid],
    ) ?? null
  );
}

// Opens a document, selecting its local copy. Local edits not sent yet are
// left alone: sending them will find out if the backend moved on.
export async function openDocument(
  document: BackendDocument,
): Promise<Model | null> {
  const uuid = localCopyOf(document.id);
  if (uuid) {
    const link = readLinks()[uuid];
    if (link.version !== document.version && !link.dirty) {
      await replaceLocalCopy(uuid, document.id);
    }
    return selectModelFromStorage(uuid);
  }
  const { model, version } = await downloadDocument(document.id);
  model.setName(document.name);
  await saveModelToStorage(model);
  const selected = getSelectedUuid();
  if (selected) {
    const links = readLinks();
    links[selected] = {
      documentId: document.id,
      version,
      dirty: false,
      paused: false,
    };
    writeLinks(links);
  }
  return model;
}

async function replaceLocalCopy(
  uuid: string,
  documentId: string,
): Promise<void> {
  const { model, version } = await downloadDocument(documentId);
  await saveModelBytesInStorage(uuid, model);
  updateLink(uuid, { version, dirty: false, paused: false });
}

// Throws the local edits away for what the backend has.
export async function takeBackendVersion(uuid: string): Promise<Model | null> {
  const link = readLinks()[uuid];
  if (!link) {
    return null;
  }
  await replaceLocalCopy(uuid, link.documentId);
  return uuid === getSelectedUuid() ? selectModelFromStorage(uuid) : null;
}

// Stores the open workbook as a new document and keeps it in step with it.
export async function uploadSelected(model: Model): Promise<void> {
  const uuid = getSelectedUuid();
  if (!uuid) {
    return;
  }
  const bytes = model.toBytes();
  const response = await request(
    `?name=${encodeURIComponent(model.getName())}`,
    {
      method: "POST",
      headers: { "Content-Type": "application/octet-stream" },
      body: new Uint8Array(bytes),
    },
  );
  if (!response.ok) {
    throw failed(response);
  }
  const document = (await response.json()) as BackendDocument;
  const links = readLinks();
  links[uuid] = {
    documentId: document.id,
    version: document.version,
    dirty: false,
    paused: false,
  };
  writeLinks(links);
}

export async function deleteDocument(id: string): Promise<void> {
  const response = await request(`/${encodeURIComponent(id)}`, {
    method: "DELETE",
  });
  if (!response.ok && response.status !== 404) {
    throw failed(response);
  }
  const uuid = localCopyOf(id);
  if (uuid) {
    removeLink(uuid);
  }
}

export interface DocumentSnapshot {
  version: number;
  sheets: { name: string; cells: unknown[] }[];
}

export async function readDocumentSnapshot(
  id: string,
): Promise<DocumentSnapshot> {
  const response = await request(`/${encodeURIComponent(id)}/snapshot`);
  if (!response.ok) {
    throw failed(response);
  }
  return (await response.json()) as DocumentSnapshot;
}

// Which document the open workbook is, if it is on the backend.
export function getSelectedDocumentId(): string | null {
  const uuid = getSelectedUuid();
  return uuid ? (readLinks()[uuid]?.documentId ?? null) : null;
}

export function isSelectedOnBackend(): boolean {
  const uuid = getSelectedUuid();
  return uuid !== null && readLinks()[uuid] !== undefined;
}

// Whether the open workbook has edits the backend refused.
export function isSelectedPaused(): boolean {
  const uuid = getSelectedUuid();
  return uuid !== null && readLinks()[uuid]?.paused === true;
}

export function markSelectedDirty(): void {
  const uuid = getSelectedUuid();
  if (uuid && readLinks()[uuid]) {
    updateLink(uuid, { dirty: true });
  }
}

export interface DocumentConflict {
  uuid: string;
  name: string;
}

let sending = false;

// Sends every workbook with edits the backend hasn't got. Returns the first
// one the backend refused because it has moved on.
export async function sendEdits(
  openModel: Model,
): Promise<DocumentConflict | null> {
  if (sending || !getToken()) {
    return null;
  }
  sending = true;
  let conflict: DocumentConflict | null = null;
  try {
    const selected = getSelectedUuid();
    for (const [uuid, link] of Object.entries(readLinks())) {
      if (!link.dirty || link.paused || !getModelsMetadata()[uuid]) {
        continue;
      }
      const model =
        uuid === selected ? openModel : await loadModelFromStorage(uuid);
      if (!model) {
        continue;
      }
      // Cleared first, so that an edit made while this is sent counts
      updateLink(uuid, { dirty: false });
      let response: Response;
      try {
        response = await request(
          `/${encodeURIComponent(link.documentId)}?version=${link.version}&name=${encodeURIComponent(model.getName())}`,
          {
            method: "PUT",
            headers: { "Content-Type": "application/octet-stream" },
            body: new Uint8Array(model.toBytes()),
          },
        );
      } catch {
        // Offline or the token is wrong: try again next time
        updateLink(uuid, { dirty: true });
        break;
      }
      if (response.ok) {
        const { version } = (await response.json()) as { version: number };
        updateLink(uuid, { version });
      } else if (response.status === 404) {
        removeLink(uuid);
      } else if (response.status === 409) {
        updateLink(uuid, { dirty: true, paused: true });
        conflict ??= { uuid, name: model.getName() };
      } else {
        updateLink(uuid, { dirty: true });
      }
    }
  } finally {
    sending = false;
  }
  return conflict;
}
