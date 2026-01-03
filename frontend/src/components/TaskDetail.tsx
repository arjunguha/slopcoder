import {
  createSignal,
  createResource,
  createEffect,
  onCleanup,
  For,
  Show,
} from "solid-js";
import { useParams, A } from "@solidjs/router";
import { getTask, sendPrompt, subscribeToTask } from "../api/client";
import type { Task, CodexEvent, CompletedItem } from "../types";

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

function EventDisplay(props: { event: CodexEvent }) {
  const e = props.event;

  if (e.type === "thread.started") {
    return (
      <div class="text-xs text-gray-500 dark:text-gray-400">Session started: {e.thread_id}</div>
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

  return null;
}

function ItemDisplay(props: { item: CompletedItem }) {
  const item = props.item;

  if (item.type === "reasoning") {
    return (
      <div class="text-gray-600 dark:text-gray-400 italic text-sm py-1">
        <span class="text-purple-600 dark:text-purple-400">Thinking:</span> {item.text}
      </div>
    );
  }

  if (item.type === "agent_message") {
    return (
      <div class="text-gray-900 dark:text-white py-1">
        <span class="text-green-600 dark:text-green-400">Agent:</span> {item.text}
      </div>
    );
  }

  if (item.type === "tool_call") {
    return (
      <div class="text-yellow-600 dark:text-yellow-400 font-mono text-sm py-1">
        <span class="text-yellow-700 dark:text-yellow-500">Tool:</span> {item.name}
        {item.arguments && (
          <pre class="text-xs text-gray-600 dark:text-gray-400 mt-1 overflow-x-auto">
            {item.arguments}
          </pre>
        )}
      </div>
    );
  }

  if (item.type === "tool_output") {
    return (
      <div class="text-gray-700 dark:text-gray-300 font-mono text-xs py-1 bg-gray-200 dark:bg-gray-800 rounded p-2 my-1 overflow-x-auto">
        <pre>{item.output}</pre>
      </div>
    );
  }

  return (
    <div class="text-gray-500 text-sm">
      [{item.type}] {item.text || JSON.stringify(item)}
    </div>
  );
}

export default function TaskDetail() {
  const params = useParams();
  const [task, { refetch }] = createResource(() => params.id, getTask);
  const [events, setEvents] = createSignal<CodexEvent[]>([]);
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
          <h1 class="text-xl font-bold text-gray-900 dark:text-gray-100 flex-1">{task()!.name}</h1>
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
        {/* Task Info */}
        <div class="text-sm text-gray-500 dark:text-gray-400 mb-4">
          <span class="font-medium text-gray-700 dark:text-gray-300">{task()!.environment}</span>
          <span class="mx-2">•</span>
          <span>{task()!.branch}</span>
          {task()!.session_id && (
            <>
              <span class="mx-2">•</span>
              <span class="font-mono text-xs">{task()!.session_id}</span>
            </>
          )}
        </div>

        {/* History */}
        <div class="mb-4">
          <h2 class="text-sm font-medium text-gray-500 dark:text-gray-400 mb-2">History</h2>
          <div class="space-y-2">
            <For each={task()!.history}>
              {(run, index) => (
                <div class="p-3 bg-gray-100 dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700">
                  <div class="flex items-center justify-between mb-1">
                    <span class="text-xs text-gray-500">
                      Prompt {index() + 1}
                    </span>
                    <span
                      class={`text-xs ${
                        run.success === true
                          ? "text-green-600 dark:text-green-400"
                          : run.success === false
                          ? "text-red-600 dark:text-red-400"
                          : "text-blue-600 dark:text-blue-400"
                      }`}
                    >
                      {run.success === true
                        ? "✓ Success"
                        : run.success === false
                        ? "✗ Failed"
                        : "Running..."}
                    </span>
                  </div>
                  <p class="text-gray-800 dark:text-gray-200">{run.prompt}</p>
                </div>
              )}
            </For>
          </div>
        </div>

        {/* Live Output */}
        <Show when={events().length > 0 || task()!.status === "running"}>
          <div class="flex-1 flex flex-col min-h-0 mb-4">
            <h2 class="text-sm font-medium text-gray-500 dark:text-gray-400 mb-2">Live Output</h2>
            <div
              ref={outputRef}
              class="flex-1 bg-gray-100 dark:bg-gray-900 rounded-lg p-4 font-mono text-sm overflow-y-auto border border-gray-200 dark:border-gray-700"
              style="max-height: 400px"
            >
              <For each={events()}>{(event) => <EventDisplay event={event} />}</For>
              <Show when={task()!.status === "running" && events().length === 0}>
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
            <div class="mb-4 p-3 bg-red-100 dark:bg-red-900/50 text-red-700 dark:text-red-200 rounded-lg text-sm border border-red-300 dark:border-red-700">
              {error()}
            </div>
          </Show>

          <form onSubmit={handleSendPrompt} class="flex gap-2">
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
      </Show>
    </div>
  );
}
