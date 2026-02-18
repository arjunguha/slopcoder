export const TASK_MESSAGE_DRAFTS_STORAGE_KEY = "slopcoderTaskMessageDrafts";

type KeyValueStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;

type TaskMessageDrafts = Record<string, string>;

function resolveStorage(storage?: KeyValueStorage | null): KeyValueStorage | null {
  if (storage !== undefined) {
    return storage;
  }
  if (typeof window === "undefined") {
    return null;
  }
  return window.localStorage;
}

function readDrafts(storage: KeyValueStorage): TaskMessageDrafts {
  let raw: string | null = null;
  try {
    raw = storage.getItem(TASK_MESSAGE_DRAFTS_STORAGE_KEY);
  } catch {
    return {};
  }
  if (!raw) {
    return {};
  }

  try {
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return {};
    }
    const drafts: TaskMessageDrafts = {};
    for (const [taskId, value] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof value === "string") {
        drafts[taskId] = value;
      }
    }
    return drafts;
  } catch {
    return {};
  }
}

export function getTaskMessageDraft(taskId: string, storage?: KeyValueStorage | null): string {
  const activeStorage = resolveStorage(storage);
  if (!activeStorage) {
    return "";
  }

  const drafts = readDrafts(activeStorage);
  return drafts[taskId] ?? "";
}

export function setTaskMessageDraft(taskId: string, value: string, storage?: KeyValueStorage | null): void {
  const activeStorage = resolveStorage(storage);
  if (!activeStorage) {
    return;
  }

  const drafts = readDrafts(activeStorage);
  if (value === "") {
    delete drafts[taskId];
  } else {
    drafts[taskId] = value;
  }

  if (Object.keys(drafts).length === 0) {
    try {
      activeStorage.removeItem(TASK_MESSAGE_DRAFTS_STORAGE_KEY);
    } catch {
      // Ignore local cache write failures and keep UI responsive.
    }
    return;
  }

  try {
    activeStorage.setItem(TASK_MESSAGE_DRAFTS_STORAGE_KEY, JSON.stringify(drafts));
  } catch {
    // Ignore local cache write failures and keep UI responsive.
  }
}
