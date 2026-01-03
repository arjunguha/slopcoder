import {
  createSignal,
  createResource,
  createEffect,
  createMemo,
  onCleanup,
  For,
  Show,
} from "solid-js";
import { useParams, A } from "@solidjs/router";
import { getTask, sendPrompt, subscribeToTask, getTaskOutput, getTaskDiff } from "../api/client";
import type { Task, AgentEvent, CompletedItem } from "../types";
import { marked } from "marked";
import DOMPurify from "dompurify";

function StatusBadge(props: { status: Task["status"] }) {
  const colors = {
    pending: "bg-gray-500",
    running: "bg-blue-500 animate-pulse",
    completed: "bg-green-500",
    failed: "bg-red-500",
  };

  return (
    <span
      class={`inline-block px-3 py-1 text-sm font-medium text-white rounded-full ${colors[props.status]}`}
    >
      {props.status}
    </span>
  );
}

function renderPrettyJson(value?: string) {
  if (!value) return value;
  try {
    const parsed = JSON.parse(value);
    return JSON.stringify(parsed, null, 2);
  } catch {
    return value;
  }
}

function formatPath(path: string) {
  const parts = path.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 2) return path;
  return parts.slice(parts.length - 2).join("/");
}

function summarizeOutput(output?: string) {
  if (!output) return "No output";
  const lines = output.split(/\r?\n/);
  const lineCount = lines[lines.length - 1] === "" ? lines.length - 1 : lines.length;
  const size = output.length;
  return `${lineCount} line${lineCount === 1 ? "" : "s"}, ${size} char${size === 1 ? "" : "s"}`;
}

function summarizeJsonShape(value?: string) {
  if (!value) return null;
  try {
    const parsed = JSON.parse(value);
    if (Array.isArray(parsed)) {
      return `JSON array (${parsed.length} item${parsed.length === 1 ? "" : "s"})`;
    }
    if (parsed && typeof parsed === "object") {
      const keys = Object.keys(parsed);
      const preview = keys.slice(0, 4).join(", ");
      return `JSON object (${keys.length} key${keys.length === 1 ? "" : "s"}${preview ? `: ${preview}` : ""})`;
    }
    return `JSON ${typeof parsed}`;
  } catch {
    return null;
  }
}

function CommandExecutionSummary(props: { item: CompletedItem }) {
  const command = props.item.command || "Command";
  const status = props.item.status || "completed";
  const exitCode = props.item.exit_code;
  const summary = summarizeOutput(props.item.aggregated_output || props.item.output);
  return (
    <div class="text-sm text-gray-700 dark:text-gray-300 py-1">
      <div class="font-medium text-gray-900 dark:text-gray-100">
        Ran: <span class="font-mono">{command}</span>
      </div>
      <div class="text-xs text-gray-500 dark:text-gray-400">
        Status: {status}
        {exitCode !== undefined && ` (exit ${exitCode})`}
        <span class="mx-2">•</span>
        Output: {summary}
      </div>
    </div>
  );
}

function FileChangeSummary(props: { item: CompletedItem }) {
  const changes = props.item.changes || [];
  const byKind = changes.reduce<Record<string, number>>((acc, change) => {
    acc[change.kind] = (acc[change.kind] || 0) + 1;
    return acc;
  }, {});
  const kinds = Object.keys(byKind)
    .map((kind) => `${byKind[kind]} ${kind}`)
    .join(", ");
  const files = changes.slice(0, 3).map((change) => formatPath(change.path));
  return (
    <div class="text-sm text-gray-700 dark:text-gray-300 py-1">
      <div class="font-medium text-gray-900 dark:text-gray-100">
        Files changed: {changes.length}
        {kinds ? ` (${kinds})` : ""}
      </div>
      {files.length > 0 && (
        <div class="text-xs text-gray-500 dark:text-gray-400">
          {files.join(", ")}
          {changes.length > files.length && "…"}
        </div>
      )}
    </div>
  );
}

function ToolOutputSummary(props: { item: CompletedItem }) {
  const output = props.item.output || "";
  const jsonShape = summarizeJsonShape(output);
  const summary = summarizeOutput(output);
  return (
    <div class="text-sm text-gray-700 dark:text-gray-300 py-1">
      <div class="font-medium text-gray-900 dark:text-gray-100">Tool output</div>
      <div class="text-xs text-gray-500 dark:text-gray-400">
        {jsonShape ? `${jsonShape} • ` : ""}
        {summary}
      </div>
    </div>
  );
}

function MarkdownBlock(props: { content?: string }) {
  const html = createMemo(() => {
    const raw = props.content || "";
    const parsed = marked.parse(raw, { breaks: true });
    const htmlString = typeof parsed === "string" ? parsed : "";
    return DOMPurify.sanitize(htmlString);
  });

  return <div class="markdown" innerHTML={html()} />;
}

function EventDisplay(props: { event: AgentEvent }) {
  const e = props.event;

  if (e.type === "session.started") {
    return (
      <div class="text-xs text-gray-500 dark:text-gray-400">Session started: {e.session_id}</div>
    );
  }

  if (e.type === "turn.started") {
    return <div class="text-xs text-blue-600 dark:text-blue-400">Turn started...</div>;
  }

  if (e.type === "item.completed") {
    return <ItemDisplay item={e.item} />;
  }

  if (e.type === "turn.completed") {
    return (
      <div class="text-xs text-gray-500 dark:text-gray-400 border-t border-gray-300 dark:border-gray-700 pt-2 mt-2">
        Turn completed
        {e.usage && (
          <span class="ml-2">
            ({e.usage.input_tokens} in / {e.usage.output_tokens} out)
          </span>
        )}
      </div>
    );
  }

  if (e.type === "prompt.sent") {
    return (
      <div class="py-2">
        <div class="text-xs text-gray-500 dark:text-gray-400 mb-1">Prompt</div>
        <div class="bg-blue-50 dark:bg-blue-900/30 border border-blue-200 dark:border-blue-800 rounded-lg px-3 py-2 text-sm text-gray-900 dark:text-gray-100">
          {e.prompt}
        </div>
      </div>
    );
  }

  return null;
}

function ItemDisplay(props: { item: CompletedItem }) {
  const item = props.item;

  if (item.type === "reasoning") {
    return (
      <div class="text-gray-600 dark:text-gray-400 italic text-sm py-1">
        <span class="text-purple-600 dark:text-purple-400">Thinking:</span>{" "}
        <span>{item.text}</span>
      </div>
    );
  }

  if (item.type === "agent_message") {
    return (
      <div class="text-gray-900 dark:text-white py-1">
        <div class="text-green-600 dark:text-green-400 mb-1">Agent</div>
        <MarkdownBlock content={item.text} />
      </div>
    );
  }

  if (item.type === "tool_call") {
    return (
      <div class="text-yellow-600 dark:text-yellow-400 font-mono text-sm py-1">
        <span class="text-yellow-700 dark:text-yellow-500">Tool:</span> {item.name}
        {item.arguments && (
          <pre class="text-xs text-gray-600 dark:text-gray-400 mt-1 overflow-x-auto">
            {renderPrettyJson(item.arguments)}
          </pre>
        )}
      </div>
    );
  }

  if (item.type === "tool_output") {
    return (
      <ToolOutputSummary item={item} />
    );
  }

  if (item.type === "command_execution") {
    return <CommandExecutionSummary item={item} />;
  }

  if (item.type === "file_change") {
    return <FileChangeSummary item={item} />;
  }

  return (
    <div class="text-gray-600 dark:text-gray-400 text-sm py-1">
      <div class="font-medium text-gray-900 dark:text-gray-100">
        Event: {item.type}
      </div>
      {item.text && <div class="text-xs text-gray-500 dark:text-gray-400">{item.text}</div>}
    </div>
  );
}

export default function TaskDetail() {
  const params = useParams();
  const [task, { refetch }] = createResource(() => params.id, getTask);
  const [persistedOutput, { refetch: refetchOutput }] = createResource(
    () => params.id,
    async (id) => (await getTaskOutput(id)).events
  );
  const [events, setEvents] = createSignal<AgentEvent[]>([]);
  const combinedEvents = createMemo(() => [
    ...(persistedOutput() || []),
    ...events(),
  ]);
  const [diff, { refetch: refetchDiff }] = createResource(
    () => params.id,
    getTaskDiff
  );
  const [newPrompt, setNewPrompt] = createSignal("");
  const [sending, setSending] = createSignal(false);
  const [error, setError] = createSignal("");

  let outputRef: HTMLDivElement | undefined;

  // Subscribe to WebSocket when task is running
  createEffect(() => {
    const t = task();
    if (t?.status === "running") {
      const unsubscribe = subscribeToTask(
        t.id,
        (event) => {
          setEvents((prev) => [...prev, event]);
          // Auto-scroll to bottom
          setTimeout(() => {
            if (outputRef) {
              outputRef.scrollTop = outputRef.scrollHeight;
            }
          }, 10);
        },
        () => {
          // On close, refetch task to get final status
          setTimeout(() => refetch(), 500);
          setTimeout(() => refetchOutput(), 500);
          setEvents([]);
          setTimeout(() => refetchDiff(), 500);
        }
      );

      onCleanup(unsubscribe);
    }
  });

  // Poll for updates when running
  createEffect(() => {
    if (task()?.status === "running") {
      const interval = setInterval(() => refetch(), 3000);
      onCleanup(() => clearInterval(interval));
    }
  });

  const handleSendPrompt = async (e: Event) => {
    e.preventDefault();
    if (!newPrompt().trim() || sending()) return;

    setError("");
    setSending(true);
    setEvents([]); // Clear old events

    try {
      await sendPrompt(params.id!, { prompt: newPrompt() });
      setNewPrompt("");
      refetch();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to send prompt");
    } finally {
      setSending(false);
    }
  };

  return (
    <div class="h-full flex flex-col">
      {/* Header */}
      <div class="flex items-center gap-4 mb-4">
        <A href="/" class="text-gray-600 dark:text-gray-400 hover:text-gray-900 dark:hover:text-gray-200">
          ← Back
        </A>
        <Show when={task()}>
          <h1 class="text-xl font-bold text-gray-900 dark:text-gray-100 flex-1">{task()!.feature_branch}</h1>
          <StatusBadge status={task()!.status} />
        </Show>
      </div>

      <Show when={task.loading}>
        <div class="text-gray-500 dark:text-gray-400">Loading task...</div>
      </Show>

      <Show when={task.error}>
        <div class="p-4 bg-red-100 dark:bg-red-900/50 text-red-700 dark:text-red-200 rounded-lg border border-red-300 dark:border-red-700">
          Error: {task.error?.message}
        </div>
      </Show>

      <Show when={task()}>
        <div class="flex flex-col gap-6 lg:flex-row min-h-0">
          <div class="flex-1 min-w-0 flex flex-col">
            {/* Task Info */}
            <div class="text-sm text-gray-500 dark:text-gray-400 mb-4">
              <span class="font-medium text-gray-700 dark:text-gray-300">{task()!.environment}</span>
              <span class="mx-2">•</span>
              <span>
                base: {task()!.base_branch || "unknown"}
              </span>
              <span class="mx-2">•</span>
              <span>agent: {task()!.agent}</span>
              <span class="mx-2">•</span>
              <span>
                feature: {task()!.feature_branch}
              </span>
              {task()!.session_id && (
                <>
                  <span class="mx-2">•</span>
                  <span class="font-mono text-xs">{task()!.session_id}</span>
                </>
              )}
            </div>

            {/* Output */}
            <Show when={combinedEvents().length > 0 || task()!.status === "running"}>
              <div class="flex-1 flex flex-col min-h-0">
                <h2 class="text-sm font-medium text-gray-500 dark:text-gray-400 mb-2">Output</h2>
                <Show when={persistedOutput.error}>
                  <div class="text-sm text-red-600 dark:text-red-400 mb-2">
                    Failed to load saved output: {persistedOutput.error?.message}
                  </div>
                </Show>
                <div
                  ref={outputRef}
                  class="flex-1 bg-gray-100 dark:bg-gray-900 rounded-lg p-4 text-sm overflow-y-auto border border-gray-200 dark:border-gray-700"
                  style="max-height: 480px"
                >
                  <For each={combinedEvents()}>{(event) => <EventDisplay event={event} />}</For>
                  <Show when={task()!.status === "running" && combinedEvents().length === 0}>
                    <div class="text-gray-500 animate-pulse">
                      Waiting for output...
                    </div>
                  </Show>
                </div>
              </div>
            </Show>

            {/* New Prompt Form */}
            <Show when={task()!.status !== "running"}>
              <Show when={error()}>
                <div class="mt-4 mb-4 p-3 bg-red-100 dark:bg-red-900/50 text-red-700 dark:text-red-200 rounded-lg text-sm border border-red-300 dark:border-red-700">
                  {error()}
                </div>
              </Show>

              <form onSubmit={handleSendPrompt} class="flex gap-2 mt-4">
                <input
                  type="text"
                  value={newPrompt()}
                  onInput={(e) => setNewPrompt(e.currentTarget.value)}
                  placeholder="Send a follow-up prompt..."
                  class="flex-1 px-4 py-2 bg-white dark:bg-gray-800 border border-gray-300 dark:border-gray-700 rounded-lg text-gray-900 dark:text-gray-100 placeholder-gray-400 dark:placeholder-gray-500 focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                />
                <button
                  type="submit"
                  disabled={!newPrompt().trim() || sending()}
                  class="px-6 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                >
                  {sending() ? "Sending..." : "Send"}
                </button>
              </form>
            </Show>
          </div>

          {/* Git Diff */}
          <div class="w-full lg:w-1/3 xl:w-1/2">
            <h2 class="text-sm font-medium text-gray-500 dark:text-gray-400 mb-2">Diff vs Base Branch</h2>
            <Show when={diff.loading}>
              <div class="text-gray-500 dark:text-gray-400">Loading diff...</div>
            </Show>
            <Show when={diff.error}>
              <div class="text-sm text-red-600 dark:text-red-400">
                Failed to load diff: {diff.error?.message}
              </div>
            </Show>
            <Show when={diff()}>
              <div
                class="bg-gray-100 dark:bg-gray-900 rounded-lg p-4 font-mono text-xs overflow-x-auto overflow-y-auto border border-gray-200 dark:border-gray-700 whitespace-pre"
                style="max-height: 480px"
              >
                {diff()!.diff.trim() || "No changes yet."}
              </div>
            </Show>
          </div>
        </div>
      </Show>
    </div>
  );
}
