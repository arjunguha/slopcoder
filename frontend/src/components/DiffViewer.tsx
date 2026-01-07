import { For, Show, createMemo } from "solid-js";

interface DiffLine {
  type: "add" | "remove" | "context" | "header";
  content: string;
}

interface DiffViewerProps {
  staged: string;
  unstaged: string;
}

function parseDiff(diff: string): DiffLine[] {
  if (!diff.trim()) return [];

  return diff.split("\n").map((line) => {
    if (line.startsWith("+") && !line.startsWith("+++")) {
      return { type: "add", content: line };
    } else if (line.startsWith("-") && !line.startsWith("---")) {
      return { type: "remove", content: line };
    } else if (
      line.startsWith("@@") ||
      line.startsWith("diff ") ||
      line.startsWith("index ") ||
      line.startsWith("---") ||
      line.startsWith("+++")
    ) {
      return { type: "header", content: line };
    } else {
      return { type: "context", content: line };
    }
  });
}

function DiffSection(props: { lines: DiffLine[]; isStaged: boolean }) {
  return (
    <For each={props.lines}>
      {(line) => {
        // Color scheme:
        // - Staged (dark): dark-green for adds, dark-red for removes
        // - Unstaged (light): light-green for adds, light-red for removes
        let className = "";
        if (line.type === "add") {
          className = props.isStaged
            ? "text-green-700 dark:text-green-300 bg-green-100 dark:bg-green-900/50"
            : "text-green-500 dark:text-green-500 bg-green-50 dark:bg-green-950/30";
        } else if (line.type === "remove") {
          className = props.isStaged
            ? "text-red-700 dark:text-red-300 bg-red-100 dark:bg-red-900/50"
            : "text-red-500 dark:text-red-500 bg-red-50 dark:bg-red-950/30";
        } else if (line.type === "header") {
          className = props.isStaged
            ? "text-blue-700 dark:text-blue-300 font-semibold"
            : "text-blue-400 dark:text-blue-500 font-semibold";
        } else {
          className = props.isStaged
            ? "text-gray-700 dark:text-gray-300"
            : "text-gray-400 dark:text-gray-500";
        }

        return <div class={className}>{line.content || " "}</div>;
      }}
    </For>
  );
}

export function DiffViewer(props: DiffViewerProps) {
  const stagedLines = createMemo(() => parseDiff(props.staged));
  const unstagedLines = createMemo(() => parseDiff(props.unstaged));

  const hasStaged = createMemo(() => props.staged.trim().length > 0);
  const hasUnstaged = createMemo(() => props.unstaged.trim().length > 0);
  const hasAny = createMemo(() => hasStaged() || hasUnstaged());

  return (
    <div
      class="bg-gray-100 dark:bg-gray-900 rounded-lg p-4 font-mono text-xs overflow-x-auto overflow-y-auto border border-gray-200 dark:border-gray-700"
      style="max-height: 480px"
    >
      <Show when={!hasAny()}>
        <div class="text-gray-500 dark:text-gray-400">No changes yet.</div>
      </Show>

      <Show when={hasStaged()}>
        <div class="mb-2">
          <span class="text-xs font-semibold text-gray-600 dark:text-gray-400 bg-gray-200 dark:bg-gray-800 px-2 py-0.5 rounded">
            Staged
          </span>
        </div>
        <div class="whitespace-pre mb-4">
          <DiffSection lines={stagedLines()} isStaged={true} />
        </div>
      </Show>

      <Show when={hasUnstaged()}>
        <div class="mb-2">
          <span class="text-xs font-semibold text-gray-500 dark:text-gray-500 bg-gray-200 dark:bg-gray-800 px-2 py-0.5 rounded">
            Unstaged
          </span>
        </div>
        <div class="whitespace-pre">
          <DiffSection lines={unstagedLines()} isStaged={false} />
        </div>
      </Show>
    </div>
  );
}
