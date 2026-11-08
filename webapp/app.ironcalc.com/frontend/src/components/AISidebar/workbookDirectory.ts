import type { Model } from "@ironcalc/workbook";
import {
  createWorkbookInStorage,
  getModelsMetadata,
  getSelectedUuid,
  loadModelFromStorage,
  saveModelBytesInStorage,
} from "../storage";
import { aiLog } from "./aiLog";
import { saveVersion } from "./versions";
import type {
  WorkbookAccess,
  WorkbookInfo,
  WorkbookMode,
} from "./workbookTools";

export interface WorkbookDirectory extends WorkbookAccess {
  /** Call before each request: it decides which workbook is "the open one". */
  beginRequest: (request: string) => void;
}

function normalize(name: string): string {
  return name
    .trim()
    .replace(/\.xlsx$/i, "")
    .toLowerCase();
}

/** The saved workbook with this name, or null if there isn't exactly one. */
export function findWorkbookUuid(name: string): string | null {
  const matches = Object.entries(getModelsMetadata()).filter(
    ([, item]) => normalize(item.name) === normalize(name),
  );
  return matches.length === 1 ? matches[0][0] : null;
}

export function createWorkbookDirectory(options: {
  getActiveModel: () => Model;
  isReadOnly: () => boolean;
  onActiveChanged: () => void;
  onWorkbookChanged: (uuid: string) => void;
}): WorkbookDirectory {
  const { getActiveModel, isReadOnly, onActiveChanged, onWorkbookChanged } =
    options;
  let currentRequest = "";
  let pinnedUuid: string | null = null;
  const snapshotted = new Set<string>();

  const allNames = () =>
    Object.values(getModelsMetadata())
      .map((item) => item.name)
      .join(", ");

  // Null means the open workbook when it isn't saved, like a shared link.
  const findUuid = (name: string | undefined): string | null => {
    if (name === undefined || name.trim() === "") {
      return pinnedUuid;
    }
    const wanted = normalize(name);
    if (
      getSelectedUuid() === null &&
      normalize(getActiveModel().getName()) === wanted
    ) {
      return null;
    }
    const matches = Object.entries(getModelsMetadata()).filter(
      ([, item]) => normalize(item.name) === wanted,
    );
    if (matches.length === 0) {
      throw new Error(
        `There is no workbook named "${name}". The workbooks are: ${allNames()}.`,
      );
    }
    if (matches.length > 1) {
      throw new Error(`More than one workbook is named "${name}".`);
    }
    return matches[0][0];
  };

  // Keeps one copy per workbook for each request, taken before its first change.
  const snapshot = async (uuid: string | null, model: Model) => {
    if (uuid === null || snapshotted.has(uuid)) {
      return;
    }
    snapshotted.add(uuid);
    try {
      await saveVersion({
        uuid,
        name: getModelsMetadata()[uuid]?.name ?? model.getName(),
        request: currentRequest,
        bytes: model.toBytes(),
      });
    } catch (e) {
      aiLog("warn", "version_save_failed", { uuid, error: e });
    }
  };

  const run = async <T>(
    name: string | undefined,
    mode: WorkbookMode,
    work: (model: Model) => T,
  ): Promise<T> => {
    const uuid = findUuid(name);
    if (uuid === null || uuid === getSelectedUuid()) {
      const model = getActiveModel();
      if (mode === "write") {
        if (isReadOnly()) {
          throw new Error(
            "This workbook is read-only, so it cannot be changed.",
          );
        }
        await snapshot(uuid, model);
      }
      const result = work(model);
      if (mode === "write") {
        onActiveChanged();
      }
      return result;
    }

    // Loaded fresh each time: the user may have edited it in between.
    const model = await loadModelFromStorage(uuid);
    if (!model) {
      throw new Error(`The workbook "${name ?? uuid}" could not be opened.`);
    }
    try {
      if (mode === "write") {
        await snapshot(uuid, model);
      }
      const result = work(model);
      if (mode === "write") {
        await saveModelBytesInStorage(uuid, model);
        onWorkbookChanged(uuid);
      }
      return result;
    } finally {
      model.free();
    }
  };

  const list = (): WorkbookInfo[] => {
    const selected = getSelectedUuid();
    const saved = Object.entries(getModelsMetadata()).map(([uuid, item]) => ({
      name: item.name,
      isOpen: uuid === selected,
    }));
    return selected === null
      ? [{ name: getActiveModel().getName(), isOpen: true }, ...saved]
      : saved;
  };

  const create = async (requestedName: string): Promise<string> => {
    const name = requestedName.trim().replace(/\.xlsx$/i, "");
    if (!name) {
      throw new Error("The workbook needs a name.");
    }
    if (list().some((item) => normalize(item.name) === normalize(name))) {
      throw new Error(`A workbook named "${name}" already exists.`);
    }
    const uuid = await createWorkbookInStorage(name);
    onWorkbookChanged(uuid);
    return name;
  };

  const beginRequest = (request: string) => {
    currentRequest = request;
    // The user can switch workbooks while the assistant is still working.
    pinnedUuid = getSelectedUuid();
    snapshotted.clear();
  };

  return { run, list, create, beginRequest };
}
