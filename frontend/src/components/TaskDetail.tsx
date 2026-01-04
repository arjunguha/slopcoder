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

function parseJson(value?: string): unknown | null {
  if (!value) return null;
  try {
    return JSON.parse(value);
  } catch {
    return null;
  }
}

function renderPrettyJson(value?: string) {
  if (!value) return value;
  const parsed = parseJson(value);
  if (parsed === null) return value;
  return JSON.stringify(parsed, null, 2);
}

function formatToolCallArgs(value?: string) {
  const parsed = parseJson(value);
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    return null;
  }

  const args = parsed as Record<string, unknown>;
  const description = typeof args.description === "string" ? args.description : null;
  const command = typeof args.command === "string" ? args.command : null;
  const filePath = typeof args.file_path === "string" ? args.file_path : null;
  const url = typeof args.url === "string" ? args.url : null;
  const remaining = Object.fromEntries(
    Object.entries(args).filter(
      ([key]) => key !== "description" && key !== "command" && key !== "file_path" && key !== "url"
    )
  );

  return (
    <div class="mt-1 text-xs text-gray-600 dark:text-gray-400">
      {description && <div class="font-medium text-gray-700 dark:text-gray-300">{description}</div>}
      {command && (
        <div class="mt-1">
          <span class="uppercase tracking-wide text-gray-500 dark:text-gray-400">command</span>
          <pre class="mt-1 bg-gray-100 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md p-2 overflow-x-auto whitespace-pre-wrap">
            {command}
          </pre>
        </div>
      )}
      {filePath && (
        <div class="mt-1">
          <span class="uppercase tracking-wide text-gray-500 dark:text-gray-400">file</span>{" "}
          <span class="font-mono text-gray-800 dark:text-gray-200">{filePath}</span>
        </div>
      )}
      {url && (
        <div class="mt-1">
          <span class="uppercase tracking-wide text-gray-500 dark:text-gray-400">url</span>{" "}
          <span class="font-mono text-gray-800 dark:text-gray-200">{url}</span>
        </div>
      )}
      {Object.keys(remaining).length > 0 && (
        <pre class="mt-2 bg-gray-100 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md p-2 overflow-x-auto whitespace-pre-wrap">
          {JSON.stringify(remaining, null, 2)}
        </pre>
      )}
    </div>
  );
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

function clipText(value: string, maxLines: number, maxChars: number) {
  const lines = value.split(/\r?\n/);
  const limitedLines = lines.slice(0, maxLines);
  let clipped = lines.length > maxLines;
  let text = limitedLines.join("\n");
  if (text.length > maxChars) {
    text = text.slice(0, maxChars);
    clipped = true;
  }
  return { text, clipped };
}

function summarizeJsonShape(value?: string) {
  const parsed = parseJson(value);
  if (parsed === null) return null;
  if (Array.isArray(parsed)) {
    return `JSON array (${parsed.length} item${parsed.length === 1 ? "" : "s"})`;
  }
  if (parsed && typeof parsed === "object") {
    const keys = Object.keys(parsed);
    const preview = keys.slice(0, 4).join(", ");
    return `JSON object (${keys.length} key${keys.length === 1 ? "" : "s"}${preview ? `: ${preview}` : ""})`;
  }
  return `JSON ${typeof parsed}`;
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
  const parsed = parseJson(output);
  const parsedObject =
    parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : null;
  const stdout =
    (typeof props.item.stdout === "string" ? props.item.stdout : null) ||
    (typeof parsedObject?.stdout === "string" ? (parsedObject.stdout as string) : null);
  const stderr =
    (typeof props.item.stderr === "string" ? props.item.stderr : null) ||
    (typeof parsedObject?.stderr === "string" ? (parsedObject.stderr as string) : null);
  const exitCode =
    typeof props.item.exit_code === "number"
      ? props.item.exit_code
      : typeof parsedObject?.exit_code === "number"
        ? (parsedObject.exit_code as number)
        : undefined;
  const remaining =
    parsedObject &&
    Object.fromEntries(
      Object.entries(parsedObject).filter(
        ([key]) => key !== "stdout" && key !== "stderr" && key !== "exit_code"
      )
    );
  const remainingKeys = remaining ? Object.keys(remaining) : [];
  const outputForSummary = stdout || stderr || output;
  const jsonShape = summarizeJsonShape(output);
  const summary = summarizeOutput(outputForSummary || "");
  const summaryBits = [
    exitCode !== undefined ? `exit ${exitCode}` : null,
    jsonShape,
    summary,
  ].filter(Boolean);
  const stdoutPreview = stdout ? clipText(stdout, 12, 800) : null;
  const stderrPreview = stderr ? clipText(stderr, 12, 800) : null;
  const outputPreview = output ? clipText(output, 12, 800) : null;

  return (
    <div class="text-sm text-gray-700 dark:text-gray-300 py-1">
      <div class="font-medium text-gray-900 dark:text-gray-100">Tool output</div>
      <div class="text-xs text-gray-500 dark:text-gray-400">
        {summaryBits.join(" • ")}
      </div>
      {stdout && (
        <div class="mt-2">
          <div class="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">stdout</div>
          <pre class="text-xs bg-gray-100 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md p-2 overflow-x-auto whitespace-pre-wrap">
            {stdoutPreview?.text}
          </pre>
          {stdoutPreview?.clipped && (
            <div class="text-[11px] text-gray-500 dark:text-gray-400 mt-1">
              Output truncated in log view.
            </div>
          )}
        </div>
      )}
      {stderr && (
        <div class="mt-2">
          <div class="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">stderr</div>
          <pre class="text-xs bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-md p-2 overflow-x-auto whitespace-pre-wrap text-red-700 dark:text-red-300">
            {stderrPreview?.text}
          </pre>
          {stderrPreview?.clipped && (
            <div class="text-[11px] text-gray-500 dark:text-gray-400 mt-1">
              Output truncated in log view.
            </div>
          )}
        </div>
      )}
      {remainingKeys.length > 0 && (
        <div class="mt-2">
          <div class="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">
            details
          </div>
          <pre class="text-xs bg-gray-100 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md p-2 overflow-x-auto whitespace-pre-wrap">
            {JSON.stringify(remaining, null, 2)}
          </pre>
        </div>
      )}
      {!stdout && !stderr && remainingKeys.length === 0 && parsed !== null && (
        <div class="mt-2">
          <pre class="text-xs bg-gray-100 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md p-2 overflow-x-auto whitespace-pre-wrap">
            {JSON.stringify(parsed, null, 2)}
          </pre>
        </div>
      )}
      {!stdout && !stderr && remainingKeys.length === 0 && parsed === null && output && (
        <div class="mt-2">
          <pre class="text-xs bg-gray-100 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md p-2 overflow-x-auto whitespace-pre-wrap">
            {outputPreview?.text}
          </pre>
          {outputPreview?.clipped && (
            <div class="text-[11px] text-gray-500 dark:text-gray-400 mt-1">
              Output truncated in log view.
            </div>
          )}
        </div>
      )}
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
      <div class="text-xs text-gray-500 dark:text-gray-400">Session started</div>
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
    const formattedArgs = formatToolCallArgs(item.arguments);
    return (
      <div class="text-yellow-700 dark:text-yellow-400 text-sm py-1">
        <div class="font-medium">
          <span class="text-yellow-800 dark:text-yellow-300">Tool:</span>{" "}
          <span class="font-mono">{item.name}</span>
        </div>
        {formattedArgs}
        {!formattedArgs && item.arguments && (
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
  let formRef: HTMLFormElement | undefined;

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

  const handlePromptKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "Enter" || !e.ctrlKey) return;
    e.preventDefault();
    if (formRef?.requestSubmit) {
      formRef.requestSubmit();
    } else {
      handleSendPrompt(new Event("submit"));
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

              <form ref={formRef} onSubmit={handleSendPrompt} class="flex gap-2 mt-4">
                <textarea
                  value={newPrompt()}
                  onInput={(e) => setNewPrompt(e.currentTarget.value)}
                  onKeyDown={handlePromptKeyDown}
                  placeholder="Send a follow-up prompt... (Ctrl+Enter to send)"
                  rows={3}
                  class="flex-1 px-4 py-2 bg-white dark:bg-gray-800 border border-gray-300 dark:border-gray-700 rounded-lg text-gray-900 dark:text-gray-100 placeholder-gray-400 dark:placeholder-gray-500 focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                ></textarea>
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
