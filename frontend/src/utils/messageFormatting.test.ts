import assert from "node:assert/strict";
import {
  clipText,
  prettyPrintJsonString,
  prettyPrintJsonValue,
  summarizeJsonShape,
} from "./messageFormatting";

type TestCase = {
  name: string;
  run: () => void;
};

const tests: TestCase[] = [
  {
    name: "prettyPrintJsonString returns null on invalid JSON",
    run: () => {
      const result = prettyPrintJsonString("{nope");
      assert.equal(result, null);
    },
  },
  {
    name: "prettyPrintJsonString clips long strings",
    run: () => {
      const value = JSON.stringify({ text: "x".repeat(500) });
      const result = prettyPrintJsonString(value);
      assert.ok(result);
      assert.equal(result?.clipped, true);
      assert.ok(result?.text.includes("..."));
    },
  },
  {
    name: "prettyPrintJsonValue handles circular data",
    run: () => {
      const obj: Record<string, unknown> = { a: 1 };
      obj.self = obj;
      const result = prettyPrintJsonValue(obj);
      assert.ok(result.text.length > 0);
      assert.ok(result.clipped);
    },
  },
  {
    name: "clipText truncates by lines and characters",
    run: () => {
      const value = ["one", "two", "three", "four"].join("\n");
      const result = clipText(value, 2, 6);
      assert.ok(result.clipped);
      assert.ok(result.text.length <= 6);
    },
  },
  {
    name: "summarizeJsonShape reports arrays",
    run: () => {
      const result = summarizeJsonShape("[1,2,3]");
      assert.ok(result);
      assert.ok(result?.includes("JSON array"));
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
