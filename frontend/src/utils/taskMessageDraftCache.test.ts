import assert from "node:assert/strict";
import {
  getTaskMessageDraft,
  setTaskMessageDraft,
  TASK_MESSAGE_DRAFTS_STORAGE_KEY,
} from "./taskMessageDraftCache";

type TestCase = {
  name: string;
  run: () => void;
};

class MemoryStorage implements Pick<Storage, "getItem" | "setItem" | "removeItem"> {
  private values = new Map<string, string>();

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }
}

class ThrowingStorage implements Pick<Storage, "getItem" | "setItem" | "removeItem"> {
  getItem(_key: string): string | null {
    throw new Error("read error");
  }

  setItem(_key: string, _value: string): void {
    throw new Error("write error");
  }

  removeItem(_key: string): void {
    throw new Error("remove error");
  }
}

const tests: TestCase[] = [
  {
    name: "returns empty string when no draft exists",
    run: () => {
      const storage = new MemoryStorage();
      assert.equal(getTaskMessageDraft("task-1", storage), "");
    },
  },
  {
    name: "stores and retrieves drafts per task id",
    run: () => {
      const storage = new MemoryStorage();
      setTaskMessageDraft("task-1", "first", storage);
      setTaskMessageDraft("task-2", "second", storage);
      assert.equal(getTaskMessageDraft("task-1", storage), "first");
      assert.equal(getTaskMessageDraft("task-2", storage), "second");
    },
  },
  {
    name: "clearing one draft keeps other task drafts intact",
    run: () => {
      const storage = new MemoryStorage();
      setTaskMessageDraft("task-1", "first", storage);
      setTaskMessageDraft("task-2", "second", storage);
      setTaskMessageDraft("task-1", "", storage);
      assert.equal(getTaskMessageDraft("task-1", storage), "");
      assert.equal(getTaskMessageDraft("task-2", storage), "second");
    },
  },
  {
    name: "invalid cached JSON is ignored",
    run: () => {
      const storage = new MemoryStorage();
      storage.setItem(TASK_MESSAGE_DRAFTS_STORAGE_KEY, "{not valid json");
      assert.equal(getTaskMessageDraft("task-1", storage), "");
    },
  },
  {
    name: "non-string draft values are ignored",
    run: () => {
      const storage = new MemoryStorage();
      storage.setItem(
        TASK_MESSAGE_DRAFTS_STORAGE_KEY,
        JSON.stringify({ "task-1": 42, "task-2": "keep me" })
      );
      assert.equal(getTaskMessageDraft("task-1", storage), "");
      assert.equal(getTaskMessageDraft("task-2", storage), "keep me");
    },
  },
  {
    name: "storage read errors are handled gracefully",
    run: () => {
      const storage = new ThrowingStorage();
      assert.equal(getTaskMessageDraft("task-1", storage), "");
    },
  },
  {
    name: "storage write errors are handled gracefully",
    run: () => {
      const storage = new ThrowingStorage();
      setTaskMessageDraft("task-1", "value", storage);
      setTaskMessageDraft("task-1", "", storage);
    },
  },
];

let failures = 0;
for (const test of tests) {
  try {
    test.run();
    // eslint-disable-next-line no-console
    console.log(`ok - ${test.name}`);
  } catch (err) {
    failures += 1;
    // eslint-disable-next-line no-console
    console.error(`not ok - ${test.name}`);
    // eslint-disable-next-line no-console
    console.error(err);
  }
}

if (failures > 0) {
  process.exit(1);
}
