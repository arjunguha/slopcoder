import assert from "node:assert/strict";
import { shouldFinalizeInitialTaskScroll } from "./taskTranscriptScroll";

type TestCase = {
  name: string;
  run: () => void;
};

const tests: TestCase[] = [
  {
    name: "returns false when initial scroll is no longer pending",
    run: () => {
      assert.equal(
        shouldFinalizeInitialTaskScroll({
          pendingInitialScroll: false,
          activeTab: "conversation",
          persistedOutputLoading: false,
          renderedEventCount: 120,
          totalEvents: 200,
        }),
        false
      );
    },
  },
  {
    name: "returns false when conversation tab is inactive",
    run: () => {
      assert.equal(
        shouldFinalizeInitialTaskScroll({
          pendingInitialScroll: true,
          activeTab: "diff",
          persistedOutputLoading: false,
          renderedEventCount: 120,
          totalEvents: 120,
        }),
        false
      );
    },
  },
  {
    name: "returns false while persisted output is still loading",
    run: () => {
      assert.equal(
        shouldFinalizeInitialTaskScroll({
          pendingInitialScroll: true,
          activeTab: "conversation",
          persistedOutputLoading: true,
          renderedEventCount: 120,
          totalEvents: 120,
        }),
        false
      );
    },
  },
  {
    name: "returns false until all event chunks are rendered",
    run: () => {
      assert.equal(
        shouldFinalizeInitialTaskScroll({
          pendingInitialScroll: true,
          activeTab: "conversation",
          persistedOutputLoading: false,
          renderedEventCount: 120,
          totalEvents: 240,
        }),
        false
      );
    },
  },
  {
    name: "returns true once loading is done and all events are rendered",
    run: () => {
      assert.equal(
        shouldFinalizeInitialTaskScroll({
          pendingInitialScroll: true,
          activeTab: "conversation",
          persistedOutputLoading: false,
          renderedEventCount: 240,
          totalEvents: 240,
        }),
        true
      );
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
