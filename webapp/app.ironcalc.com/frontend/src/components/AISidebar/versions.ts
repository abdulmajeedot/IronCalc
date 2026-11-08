const DB_NAME = "ironcalc-ai";
const STORE = "versions";
const MAX_VERSIONS_PER_WORKBOOK = 20;

export interface WorkbookVersion {
  id: number;
  uuid: string;
  name: string;
  createdAt: number;
  /** What the assistant had been asked to do when this copy was taken. */
  request: string;
}

interface StoredVersion extends WorkbookVersion {
  bytes: Uint8Array;
}

// Kept apart from the app's own database so its schema stays as upstream has it.
function openDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, 1);
    request.onupgradeneeded = () => {
      request.result
        .createObjectStore(STORE, { keyPath: "id", autoIncrement: true })
        .createIndex("uuid", "uuid");
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

function result<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function allForWorkbook(
  db: IDBDatabase,
  uuid: string,
): Promise<StoredVersion[]> {
  const index = db.transaction(STORE).objectStore(STORE).index("uuid");
  const versions = await result(
    index.getAll(uuid) as IDBRequest<StoredVersion[]>,
  );
  return versions.sort((a, b) => b.id - a.id);
}

export async function saveVersion(version: {
  uuid: string;
  name: string;
  request: string;
  bytes: Uint8Array;
}): Promise<void> {
  const db = await openDatabase();
  try {
    await result(
      db
        .transaction(STORE, "readwrite")
        .objectStore(STORE)
        .add({ ...version, createdAt: Date.now() }),
    );
    const stale = (await allForWorkbook(db, version.uuid)).slice(
      MAX_VERSIONS_PER_WORKBOOK,
    );
    if (stale.length > 0) {
      const store = db.transaction(STORE, "readwrite").objectStore(STORE);
      await Promise.all(stale.map((item) => result(store.delete(item.id))));
    }
  } finally {
    db.close();
  }
}

/** Newest first, without the workbook contents. */
export async function listVersions(uuid: string): Promise<WorkbookVersion[]> {
  const db = await openDatabase();
  try {
    return (await allForWorkbook(db, uuid)).map(
      ({ bytes: _bytes, ...version }) => version,
    );
  } finally {
    db.close();
  }
}

export async function getVersionBytes(
  id: number,
): Promise<Uint8Array | undefined> {
  const db = await openDatabase();
  try {
    const store = db.transaction(STORE).objectStore(STORE);
    const version = await result(
      store.get(id) as IDBRequest<StoredVersion | undefined>,
    );
    return version?.bytes;
  } finally {
    db.close();
  }
}
