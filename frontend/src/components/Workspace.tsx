import {
  Show,
  For,
  createMemo,
  createResource,
  createSignal,
  createEffect,
  onCleanup,
} from "solid-js";
import { useParams } from "@solidjs/router";
import {
  listHosts,
  listEnvironments,
  createEnvironment,
  listTasks,
  createTask,
  getTask,
  getTaskOutput,
  sendPrompt,
  subscribeToTask,
  getTaskDiff,
  mergeTask,
} from "../api/client";
import {
  agentSupportsWebSearch,
  type AgentEvent,
  type AgentKind,
  type CompletedItem,
  type Task,
} from "../types";
import { DiffViewer } from "./DiffViewer";
import { marked } from "marked";
import DOMPurify from "dompurify";

type RightMode =
  | { kind: "new-environment" }
  | { kind: "new-task"; host: string; environment: string }
  | { kind: "task"; taskId: string };

type RightTab = "conversation" | "diff";

function basenameFromPath(path: string): string {
  const normalized = path.replace(/\\/g, "/").replace(/\/+$/, "");
  const parts = normalized.split("/").filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : path;
}

function StatusBadge(props: { status: Task["status"] }) {
  const colors = {
    pending: "bg-gray-500",
    running: "bg-blue-500 animate-pulse",
    completed: "bg-green-500",
    failed: "bg-red-500",
    interrupted: "bg-amber-500",
  };

  return (
    <span
      class={`inline-block px-2 py-1 text-xs font-medium text-white rounded ${colors[props.status]}`}
    >
      {props.status}
    </span>
  );
}

function EventRow(props: { event: AgentEvent }) {
  const e = props.event;
  if (e.type === "prompt.sent") {
    return (
      <div class="rounded-lg border border-blue-200 dark:border-blue-800 bg-blue-50 dark:bg-blue-950/30 px-3 py-2">
        <div class="text-xs uppercase tracking-wide text-blue-700 dark:text-blue-300">Prompt</div>
        <div class="text-sm text-gray-900 dark:text-gray-100 whitespace-pre-wrap">{e.prompt}</div>
      </div>
    );
  }

  if (e.type === "turn.started") {
    return <div class="text-xs text-blue-600 dark:text-blue-400">Turn started</div>;
  }

  if (e.type === "session.started") {
    return <div class="text-xs text-gray-500 dark:text-gray-400">Session started</div>;
  }

  if (e.type === "turn.completed") {
    return (
      <div class="text-xs text-gray-500 dark:text-gray-400">
        Turn completed
        <Show when={e.usage}>
          <span>
            {" "}
            ({e.usage?.input_tokens ?? 0} in / {e.usage?.output_tokens ?? 0} out)
          </span>
        </Show>
      </div>
    );
  }

  if (e.type === "background_event") {
    return <div class="text-xs text-gray-500 dark:text-gray-400">Background: {e.event ?? "event"}</div>;
  }

  if (e.type === "item.completed") {
    return <CompletedItemRow item={e.item} />;
  }

  return null;
}

function CompletedItemRow(props: { item: CompletedItem }) {
  const item = props.item;

  if (item.type === "agent_message") {
    const html = createMemo(() => {
      const raw = item.text || "";
      const parsed = marked.parse(raw, { breaks: true });
      return DOMPurify.sanitize(typeof parsed === "string" ? parsed : "");
    });
    return (
      <div class="rounded-lg border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-900 px-3 py-2">
        <div class="text-xs uppercase tracking-wide text-green-700 dark:text-green-300 mb-1">Agent</div>
        <div class="markdown text-sm text-gray-900 dark:text-gray-100" innerHTML={html()} />
      </div>
    );
  }

  if (item.type === "reasoning") {
    return (
      <div class="text-sm text-gray-600 dark:text-gray-400 italic">
        Thinking: {item.text}
      </div>
    );
  }

  if (item.type === "tool_call") {
    return (
      <div class="rounded-lg border border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-900 px-3 py-2">
        <div class="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">Tool call</div>
        <div class="text-sm text-gray-900 dark:text-gray-100 font-mono">{item.name ?? "tool"}</div>
        <Show when={item.arguments}>
          <pre class="mt-1 text-xs whitespace-pre-wrap overflow-x-auto text-gray-600 dark:text-gray-300">
            {item.arguments}
          </pre>
        </Show>
      </div>
    );
  }

  if (item.type === "tool_output") {
    return (
      <div class="rounded-lg border border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-900 px-3 py-2">
        <div class="text-xs uppercase tracking-wide text-gray-500 dark:text-gray-400">Tool output</div>
        <pre class="mt-1 text-xs whitespace-pre-wrap overflow-x-auto text-gray-700 dark:text-gray-200">
          {item.output ?? ""}
        </pre>
      </div>
    );
  }

  return (
    <div class="text-xs text-gray-500 dark:text-gray-400">
      Event: {item.type}
    </div>
  );
}

function TaskPane(props: { taskId: string; activeTab: () => RightTab; hideDiff?: boolean }) {
  const [task, { refetch: refetchTask }] = createResource(() => props.taskId, getTask);
  const [persistedOutput, { refetch: refetchOutput }] = createResource(
    () => props.taskId,
    async (id) => (await getTaskOutput(id)).events
  );
  const [diff, { refetch: refetchDiff }] = createResource(
    () => (props.hideDiff ? null : props.taskId),
    async (id) => {
      if (!id) return null;
      return getTaskDiff(id);
    }
  );
  const taskData = createMemo(() => task.latest ?? task());
  const persistedEvents = createMemo(() => persistedOutput.latest ?? persistedOutput() ?? []);
  const diffData = createMemo(() => diff.latest ?? diff());

  const [liveEvents, setLiveEvents] = createSignal<AgentEvent[]>([]);
  const allEvents = createMemo(() => [...persistedEvents(), ...liveEvents()]);
  const [prompt, setPrompt] = createSignal("");
  const [sending, setSending] = createSignal(false);
  const [merging, setMerging] = createSignal(false);
  const [error, setError] = createSignal("");
  const [mergeMessage, setMergeMessage] = createSignal<{ type: "success" | "error"; text: string } | null>(null);

  let outputRef: HTMLDivElement | undefined;
  let promptRef: HTMLTextAreaElement | undefined;

  createEffect(() => {
    const t = taskData();
    if (t?.status === "running") {
      const unsubscribe = subscribeToTask(
        t.id,
        (event) => {
          setLiveEvents((prev) => [...prev, event]);
          setTimeout(() => {
            if (outputRef) outputRef.scrollTop = outputRef.scrollHeight;
          }, 10);
        },
        () => {
          setTimeout(() => refetchTask(), 300);
          setTimeout(() => refetchOutput(), 300);
          if (!props.hideDiff) {
            setTimeout(() => refetchDiff(), 300);
          }
          setLiveEvents([]);
        }
      );
      onCleanup(unsubscribe);
    }
  });

  createEffect(() => {
    const t = taskData();
    if (t?.status === "running") {
      const id = setInterval(() => refetchTask(), 3000);
      onCleanup(() => clearInterval(id));
    }
  });

  const sendFollowup = async (e: Event) => {
    e.preventDefault();
    if (!prompt().trim() || sending()) return;
    const value = prompt();
    setSending(true);
    setError("");
    setLiveEvents([{ type: "prompt.sent", prompt: value }]);
    try {
      await sendPrompt(props.taskId, { prompt: value });
      setPrompt("");
      refetchTask();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to send prompt");
    } finally {
      setSending(false);
    }
  };

  return (
    <div class="h-full flex flex-col min-h-0">
      <Show when={task.loading && !taskData()}>
        <div class="text-gray-500 dark:text-gray-400">Loading task...</div>
      </Show>
      <Show when={task.error}>
        <div class="rounded-lg border border-red-300 dark:border-red-700 bg-red-100 dark:bg-red-900/50 p-3 text-red-700 dark:text-red-200">
          {task.error?.message}
        </div>
      </Show>
      <Show when={taskData()}>
        <div class="mb-4 flex items-center justify-between gap-3">
          <div>
            <div class="text-xl font-bold text-gray-900 dark:text-gray-100">{taskData()!.name}</div>
            <div class="text-xs text-gray-500 dark:text-gray-400">
              {taskData()!.host}/{taskData()!.environment} • {taskData()!.workspace_kind} • agent: {taskData()!.agent}
            </div>
          </div>
          <div class="flex items-center gap-2">
            <Show when={taskData()!.workspace_kind === "worktree"}>
              <button
                disabled={merging() || taskData()!.status === "running"}
                onClick={async () => {
                  if (!confirm(`Merge ${taskData()!.merge_branch || taskData()!.name} into ${taskData()!.base_branch || "current"}?`)) return;
                  setMerging(true);
                  setMergeMessage(null);
                  try {
                    const res = await mergeTask(taskData()!.id);
                    setMergeMessage({ type: "success", text: res.message });
                    if (!props.hideDiff) {
                      refetchDiff();
                    }
                  } catch (err) {
                    setMergeMessage({
                      type: "error",
                      text: err instanceof Error ? err.message : "Merge failed",
                    });
                  } finally {
                    setMerging(false);
                  }
                }}
                class="rounded-md bg-purple-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-purple-700 disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {merging() ? "Merging..." : "Merge"}
              </button>
            </Show>
            <StatusBadge status={taskData()!.status} />
          </div>
        </div>

        <Show when={mergeMessage()}>
          <div
            class={`mb-3 rounded-lg border p-3 text-sm ${
              mergeMessage()!.type === "success"
                ? "border-green-300 dark:border-green-700 bg-green-100 dark:bg-green-900/50 text-green-700 dark:text-green-200"
                : "border-red-300 dark:border-red-700 bg-red-100 dark:bg-red-900/50 text-red-700 dark:text-red-200"
            }`}
          >
            {mergeMessage()!.text}
          </div>
        </Show>

        <Show when={props.activeTab() === "conversation"}>
          <div class="flex-1 min-h-0 flex flex-col">
            <div
              ref={outputRef}
              class="flex-1 min-h-0 overflow-y-auto rounded-lg border border-gray-200 dark:border-gray-700 bg-gray-100 dark:bg-gray-900 p-4 space-y-3"
            >
              <For each={allEvents()}>{(event) => <EventRow event={event} />}</For>
              <Show when={taskData()!.status === "running" && allEvents().length === 0}>
                <div class="text-gray-500 dark:text-gray-400 animate-pulse">Waiting for output...</div>
              </Show>
            </div>

            <Show when={taskData()!.status !== "running"}>
              <form onSubmit={sendFollowup} class="mt-3 flex gap-2">
                <textarea
                  ref={promptRef}
                  rows={3}
                  value={prompt()}
                  onInput={(e) => setPrompt(e.currentTarget.value)}
                  placeholder="Continue the conversation..."
                  class="flex-1 rounded-lg border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-800 px-3 py-2 text-sm text-gray-900 dark:text-gray-100"
                />
                <button
                  type="submit"
                  disabled={!prompt().trim() || sending()}
                  class="rounded-lg bg-blue-600 px-5 py-2 text-white hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed"
                >
                  {sending() ? "Sending..." : "Send"}
                </button>
              </form>
              <Show when={error()}>
                <div class="mt-2 text-sm text-red-600 dark:text-red-400">{error()}</div>
              </Show>
            </Show>
          </div>
        </Show>

        <Show when={!props.hideDiff && props.activeTab() === "diff"}>
          <div class="flex-1 min-h-0">
            <Show when={diff.loading && !diffData()}>
              <div class="text-gray-500 dark:text-gray-400">Loading diff...</div>
            </Show>
            <Show when={diff.error}>
              <div class="text-red-600 dark:text-red-400">{diff.error?.message}</div>
            </Show>
            <Show when={diffData()}>
              <DiffViewer staged={diffData()!.staged} unstaged={diffData()!.unstaged} />
            </Show>
          </div>
        </Show>
      </Show>
    </div>
  );
}

function NewTaskPane(props: {
  host: string;
  environment: string;
  onCreated: (taskId: string) => void;
}) {
  const [taskName, setTaskName] = createSignal("");
  const [useWorktree, setUseWorktree] = createSignal(false);
  const [webSearch, setWebSearch] = createSignal(false);
  const [agent, setAgent] = createSignal<AgentKind>("codex");
  const [prompt, setPrompt] = createSignal("");
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal("");
  const searchSupported = () => agentSupportsWebSearch(agent());

  let promptRef: HTMLTextAreaElement | undefined;
  createEffect(() => {
    setTimeout(() => promptRef?.focus(), 0);
  });

  const submit = async (e: Event) => {
    e.preventDefault();
    if (!prompt().trim()) return;
    setLoading(true);
    setError("");
    try {
      const task = await createTask({
        host: props.host,
        environment: props.environment,
        prompt: prompt(),
        name: taskName().trim() || undefined,
        use_worktree: useWorktree(),
        web_search: searchSupported() && webSearch(),
        agent: agent(),
      });
      props.onCreated(task.id);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create task");
    } finally {
      setLoading(false);
    }
  };

  return (
    <form onSubmit={submit} class="h-full flex flex-col min-h-0">
      <div class="flex-1 min-h-0 flex items-center justify-center">
        <div class="text-center">
          <div class="text-4xl font-extrabold tracking-tight text-gray-900 dark:text-gray-100">
            Let's Build
          </div>
          <div class="text-sm text-gray-500 dark:text-gray-400 mt-2" title={props.environment}>
            {props.host}:{basenameFromPath(props.environment)}
          </div>
          <div class="text-xs text-gray-400 dark:text-gray-500 mt-1 break-all" title={props.environment}>
            {props.environment}
          </div>
        </div>
      </div>

      <div class="mt-4">
        <div class="grid gap-3 md:grid-cols-2 mb-3">
          <input
            value={taskName()}
            onInput={(e) => setTaskName(e.currentTarget.value)}
            placeholder="Task topic (optional)"
            class="rounded-lg border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-800 px-3 py-2 text-sm text-gray-900 dark:text-gray-100"
          />
          <select
            value={agent()}
            onChange={(e) => setAgent(e.currentTarget.value as AgentKind)}
            class="rounded-lg border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-800 px-3 py-2 text-sm text-gray-900 dark:text-gray-100"
          >
            <option value="codex">Codex</option>
            <option value="claude">Claude</option>
            <option value="cursor">Cursor</option>
            <option value="gemini">Gemini</option>
            <option value="opencode">OpenCode</option>
          </select>
        </div>
        <label class="mb-3 flex items-center gap-2 text-sm text-gray-700 dark:text-gray-300">
          <input
            type="checkbox"
            checked={useWorktree()}
            onChange={(e) => setUseWorktree(e.currentTarget.checked)}
          />
          Run task in isolated worktree (mergeable)
        </label>
        <Show when={searchSupported()}>
          <label class="mb-3 flex items-center gap-2 text-sm text-gray-700 dark:text-gray-300">
            <input
              type="checkbox"
              checked={webSearch()}
              onChange={(e) => setWebSearch(e.currentTarget.checked)}
            />
            Enable web search
          </label>
        </Show>
        <textarea
          ref={promptRef}
          rows={4}
          value={prompt()}
          onInput={(e) => setPrompt(e.currentTarget.value)}
          placeholder="Describe what you want built..."
          class="w-full rounded-lg border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-800 px-3 py-2 text-sm text-gray-900 dark:text-gray-100"
        />
        <div class="mt-3 flex justify-end">
          <button
            type="submit"
            disabled={loading() || !prompt().trim()}
            class="rounded-lg bg-blue-600 px-5 py-2 text-white hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {loading() ? "Creating..." : "Create Task"}
          </button>
        </div>
        <Show when={error()}>
          <div class="mt-2 text-sm text-red-600 dark:text-red-400">{error()}</div>
        </Show>
      </div>
    </form>
  );
}

function NewEnvironmentPane(props: {
  hosts: string[];
  onCreated: (taskId: string) => void;
}) {
  const [host, setHost] = createSignal("");
  const [name, setName] = createSignal("");
  const [agent, setAgent] = createSignal<AgentKind>("codex");
  const [prompt, setPrompt] = createSignal("");
  const [useWorktree, setUseWorktree] = createSignal(false);
  const [webSearch, setWebSearch] = createSignal(false);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal("");
  const searchSupported = () => agentSupportsWebSearch(agent());

  createEffect(() => {
    if (!host() && props.hosts.length > 0) {
      setHost(props.hosts[0]);
    }
  });

  const submit = async (e: Event) => {
    e.preventDefault();
    if (!host().trim() || !name().trim() || !prompt().trim()) return;
    setLoading(true);
    setError("");
    try {
      const env = await createEnvironment({
        host: host().trim(),
        name: name().trim(),
      });
      const task = await createTask({
        host: env.host,
        environment: env.name,
        prompt: prompt(),
        use_worktree: useWorktree(),
        web_search: searchSupported() && webSearch(),
        agent: agent(),
      });
      props.onCreated(task.id);
      setName("");
      setPrompt("");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create environment");
    } finally {
      setLoading(false);
    }
  };

  return (
    <form onSubmit={submit} class="h-full flex flex-col min-h-0">
      <div class="flex-1 min-h-0 flex items-center justify-center">
        <div class="text-center">
          <div class="text-4xl font-extrabold tracking-tight text-gray-900 dark:text-gray-100">
            Let's Build
          </div>
          <div class="text-sm text-gray-500 dark:text-gray-400 mt-2">
            Create a new environment and kick off the first task immediately.
          </div>
        </div>
      </div>

      <div class="mt-4">
        <div class="grid gap-3 md:grid-cols-2 mb-3">
          <input
            list="hosts-list"
            value={host()}
            onInput={(e) => setHost(e.currentTarget.value)}
            placeholder="Host"
            class="rounded-lg border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-800 px-3 py-2 text-sm text-gray-900 dark:text-gray-100"
          />
          <datalist id="hosts-list">
            <For each={props.hosts}>{(host) => <option value={host} />}</For>
          </datalist>
          <input
            value={name()}
            onInput={(e) => setName(e.currentTarget.value)}
            placeholder="Environment name"
            class="rounded-lg border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-800 px-3 py-2 text-sm text-gray-900 dark:text-gray-100"
          />
        </div>
        <div class="grid gap-3 md:grid-cols-2 mb-3">
          <select
            value={agent()}
            onChange={(e) => setAgent(e.currentTarget.value as AgentKind)}
            class="rounded-lg border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-800 px-3 py-2 text-sm text-gray-900 dark:text-gray-100"
          >
            <option value="codex">Codex</option>
            <option value="claude">Claude</option>
            <option value="cursor">Cursor</option>
            <option value="gemini">Gemini</option>
            <option value="opencode">OpenCode</option>
          </select>
          <label class="flex items-center gap-2 text-sm text-gray-700 dark:text-gray-300">
            <input
              type="checkbox"
              checked={useWorktree()}
              onChange={(e) => setUseWorktree(e.currentTarget.checked)}
            />
            Run task in isolated worktree
          </label>
          <Show when={searchSupported()}>
            <label class="flex items-center gap-2 text-sm text-gray-700 dark:text-gray-300">
              <input
                type="checkbox"
                checked={webSearch()}
                onChange={(e) => setWebSearch(e.currentTarget.checked)}
              />
              Enable web search
            </label>
          </Show>
        </div>
        <textarea
          rows={4}
          value={prompt()}
          onInput={(e) => setPrompt(e.currentTarget.value)}
          placeholder="Initial prompt for the first task..."
          class="w-full rounded-lg border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-800 px-3 py-2 text-sm text-gray-900 dark:text-gray-100"
        />
        <div class="mt-3 flex justify-end">
          <button
            type="submit"
            disabled={loading() || !host().trim() || !name().trim() || !prompt().trim()}
            class="rounded-lg bg-blue-600 px-5 py-2 text-white hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {loading() ? "Creating..." : "Create Environment + Start Task"}
          </button>
        </div>
        <Show when={error()}>
          <div class="mt-2 text-sm text-red-600 dark:text-red-400">{error()}</div>
        </Show>
      </div>
    </form>
  );
}

export default function Workspace() {
  const params = useParams();
  const [hosts, { refetch: refetchHosts }] = createResource(listHosts);
  const [environments, { refetch: refetchEnvironments }] = createResource(listEnvironments);
  const [tasks, { refetch: refetchTasks }] = createResource(listTasks);
  const hostsData = createMemo(() => hosts.latest ?? hosts() ?? []);
  const environmentsData = createMemo(() => environments.latest ?? environments() ?? []);
  const tasksData = createMemo(() => tasks.latest ?? tasks() ?? []);
  const hostsById = createMemo(() => new Map(hostsData().map((host) => [host.host, host])));
  const hostIds = createMemo(() => hostsData().map((host) => host.host));
  const environmentsById = createMemo(() =>
    new Map(environmentsData().map((env) => [`${env.host}::${env.name}`, env]))
  );
  const environmentIds = createMemo(() => environmentsData().map((env) => `${env.host}::${env.name}`));
  const tasksById = createMemo(() => new Map(tasksData().map((task) => [task.id, task])));
  const [expanded, setExpanded] = createSignal<Record<string, boolean>>({});
  const [mode, setMode] = createSignal<RightMode>({ kind: "new-environment" });
  const [tab, setTab] = createSignal<RightTab>("conversation");
  const [mobileMenuOpen, setMobileMenuOpen] = createSignal(false);
  const [isMobile, setIsMobile] = createSignal(false);

  createEffect(() => {
    const id = setInterval(() => {
      refetchHosts();
      refetchEnvironments();
      refetchTasks();
    }, 4000);
    onCleanup(() => clearInterval(id));
  });

  createEffect(() => {
    const id = params.id;
    if (id) {
      setMode({ kind: "task", taskId: id });
      setTab("conversation");
    }
  });

  createEffect(() => {
    const query = window.matchMedia("(max-width: 1023px)");
    const apply = () => setIsMobile(query.matches);
    apply();
    query.addEventListener("change", apply);
    onCleanup(() => query.removeEventListener("change", apply));
  });

  createEffect(() => {
    if (isMobile()) {
      setTab("conversation");
    }
  });

  const tasksByEnvironment = createMemo(() => {
    const grouped: Record<string, string[]> = {};
    for (const task of tasksData()) {
      const key = `${task.host}::${task.environment}`;
      if (!grouped[key]) grouped[key] = [];
      grouped[key].push(task.id);
    }
    for (const key of Object.keys(grouped)) {
      grouped[key].sort((a, b) => {
        const left = tasksById().get(a);
        const right = tasksById().get(b);
        return +new Date(right?.created_at ?? 0) - +new Date(left?.created_at ?? 0);
      });
    }
    return grouped;
  });

  const selectedTaskId = createMemo(() => {
    const currentMode = mode();
    return currentMode.kind === "task" ? currentMode.taskId : null;
  });

  const toggleEnvironment = (name: string) => {
    setExpanded((prev) => ({ ...prev, [name]: !prev[name] }));
  };

  const sidebarContent = () => (
    <>
      <div class="mb-4">
        <h1 class="text-sm font-semibold uppercase tracking-wide text-gray-500 dark:text-gray-400">Hosts</h1>
        <Show when={hosts.loading && hostsData().length === 0}>
          <div class="text-xs text-gray-500 dark:text-gray-400 mt-1">Loading hosts...</div>
        </Show>
        <div class="mt-2 space-y-1">
          <For each={hostIds()}>
            {(hostId) => {
              const host = createMemo(() => hostsById().get(hostId));
              return (
                <div class="rounded border border-gray-200 dark:border-gray-700 px-2 py-1 text-xs text-gray-700 dark:text-gray-300">
                  <div class="font-medium">{host()?.host}</div>
                  <Show when={host() && host()!.host !== host()!.hostname}>
                    <div class="text-[11px] text-gray-500 dark:text-gray-400">{host()?.hostname}</div>
                  </Show>
                </div>
              );
            }}
          </For>
          <Show when={!hosts.loading && hostsData().length === 0}>
            <div class="text-xs text-amber-600 dark:text-amber-400">No connected slopagents.</div>
          </Show>
        </div>
      </div>

      <div class="mb-3 flex items-center justify-between">
        <h1 class="text-sm font-semibold uppercase tracking-wide text-gray-500 dark:text-gray-400">Environments</h1>
        <button
          onClick={() => {
            setMode({ kind: "new-environment" });
            setTab("conversation");
            if (isMobile()) {
              setMobileMenuOpen(false);
            }
          }}
          class="rounded-md border border-gray-300 dark:border-gray-600 px-2 py-0.5 text-xs hover:bg-gray-100 dark:hover:bg-gray-800"
          title="Create environment"
        >
          + New
        </button>
      </div>

      <Show when={environments.loading && environmentsData().length === 0}>
        <div class="text-xs text-gray-500 dark:text-gray-400">Loading environments...</div>
      </Show>

      <For each={environmentIds()}>
        {(environmentId) => {
          const env = createMemo(() => environmentsById().get(environmentId));
          return (
            <div class="mb-2">
              <div class="flex items-center justify-between px-2 py-2">
                <button
                  onClick={() => toggleEnvironment(environmentId)}
                  class="flex items-center gap-2 text-sm font-medium text-gray-800 dark:text-gray-200"
                  title={env()?.directory || env()?.name || ""}
                >
                  <span
                    class={`inline-block transition-transform ${
                      expanded()[environmentId] ? "rotate-90" : ""
                    }`}
                  >
                    ▸
                  </span>
                  {env()?.host}:{basenameFromPath(env()?.directory || env()?.name || "")}
                </button>
                <button
                  onClick={() => {
                    if (!env()) return;
                    setMode({ kind: "new-task", host: env()!.host, environment: env()!.name });
                    setTab("conversation");
                    if (isMobile()) {
                      setMobileMenuOpen(false);
                    }
                  }}
                  class="rounded-md border border-gray-300 dark:border-gray-600 px-2 py-0.5 text-xs hover:bg-gray-100 dark:hover:bg-gray-800"
                  title={`Create task in ${env()?.directory || env()?.name}`}
                >
                  +
                </button>
              </div>

              <Show when={expanded()[environmentId]}>
                <div class="pl-5 pr-1 py-1 space-y-1">
                  <For each={tasksByEnvironment()[environmentId] || []}>
                    {(taskId) => {
                      const task = createMemo(() => tasksById().get(taskId));
                      return (
                        <button
                          onClick={() => {
                            setMode({ kind: "task", taskId });
                            setTab("conversation");
                            if (isMobile()) {
                              setMobileMenuOpen(false);
                            }
                          }}
                          class={`w-full rounded-md border px-2 py-2 text-left ${
                            selectedTaskId() === taskId
                              ? "border-blue-500 bg-blue-50 dark:bg-blue-950/30"
                              : "border-transparent hover:border-gray-200 dark:hover:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800"
                          }`}
                        >
                          <div class="text-sm font-medium text-gray-900 dark:text-gray-100 truncate">
                            {task()?.name}
                          </div>
                          <div class="mt-1 flex items-center justify-between">
                            <span class="text-[11px] text-gray-500 dark:text-gray-400">
                              {new Date(task()?.created_at || "").toLocaleDateString()}
                            </span>
                            <Show when={task()}>
                              <StatusBadge status={task()!.status} />
                            </Show>
                          </div>
                        </button>
                      );
                    }}
                  </For>
                  <Show when={(tasksByEnvironment()[environmentId] || []).length === 0}>
                    <div class="text-xs text-gray-500 dark:text-gray-400 px-2 py-1">No tasks yet.</div>
                  </Show>
                </div>
              </Show>
            </div>
          );
        }}
      </For>
    </>
  );

  return (
    <div class="h-screen bg-white dark:bg-gray-950 text-gray-900 dark:text-gray-100">
      <div class="lg:hidden flex items-center justify-between px-3 py-2 border-b border-gray-200 dark:border-gray-800">
        <button
          onClick={() => setMobileMenuOpen(true)}
          class="rounded-md border border-gray-300 dark:border-gray-700 px-2 py-1 text-sm"
          title="Open environments"
        >
          ☰
        </button>
        <div class="text-sm font-semibold text-gray-800 dark:text-gray-200">Slopcoder</div>
        <div class="w-7" />
      </div>

      <div class="h-[calc(100vh-41px)] lg:h-full lg:grid lg:grid-cols-[320px_1fr]">
        <aside class="hidden lg:block border-r border-gray-200 dark:border-gray-800 bg-gray-50 dark:bg-gray-900/50 p-3 overflow-y-auto">
          {sidebarContent()}
        </aside>

        <Show when={isMobile() && mobileMenuOpen()}>
          <div class="fixed inset-0 z-50 lg:hidden">
            <button
              class="absolute inset-0 bg-black/40"
              onClick={() => setMobileMenuOpen(false)}
              aria-label="Close menu"
            />
            <aside class="absolute left-0 top-0 h-full w-[88%] max-w-sm border-r border-gray-200 dark:border-gray-800 bg-gray-50 dark:bg-gray-900 p-3 overflow-y-auto">
              <div class="mb-3 flex items-center justify-between">
                <div class="text-sm font-semibold text-gray-800 dark:text-gray-200">Navigation</div>
                <button
                  onClick={() => setMobileMenuOpen(false)}
                  class="rounded-md border border-gray-300 dark:border-gray-700 px-2 py-1 text-xs"
                >
                  Close
                </button>
              </div>
              {sidebarContent()}
            </aside>
          </div>
        </Show>

        <main class="p-3 lg:p-5 min-h-0 flex flex-col">
          <Show when={mode().kind === "task"}>
            <div class="mb-4 hidden lg:flex gap-2 border-b border-gray-200 dark:border-gray-800">
              <button
                onClick={() => setTab("conversation")}
                class={`px-3 py-2 text-sm font-medium ${
                  tab() === "conversation"
                    ? "text-blue-600 dark:text-blue-400 border-b-2 border-blue-600 dark:border-blue-400"
                    : "text-gray-500 dark:text-gray-400"
                }`}
              >
                Conversation
              </button>
              <Show when={!isMobile()}>
                <button
                  onClick={() => setTab("diff")}
                  class={`px-3 py-2 text-sm font-medium ${
                    tab() === "diff"
                      ? "text-blue-600 dark:text-blue-400 border-b-2 border-blue-600 dark:border-blue-400"
                      : "text-gray-500 dark:text-gray-400"
                  }`}
                >
                  Diff
                </button>
              </Show>
            </div>
          </Show>

          <div class="flex-1 min-h-0">
            <Show when={mode().kind === "new-environment"}>
              <NewEnvironmentPane
                hosts={hostIds()}
                onCreated={(taskId) => {
                  refetchEnvironments();
                  refetchTasks();
                  setMode({ kind: "task", taskId });
                  setTab("conversation");
                }}
              />
            </Show>

            <Show when={mode().kind === "new-task"}>
              <NewTaskPane
                host={(mode() as { kind: "new-task"; host: string; environment: string }).host}
                environment={(mode() as { kind: "new-task"; host: string; environment: string }).environment}
                onCreated={(taskId) => {
                  refetchTasks();
                  setMode({ kind: "task", taskId });
                  setTab("conversation");
                }}
              />
            </Show>

            <Show when={mode().kind === "task"}>
              <TaskPane
                taskId={(mode() as { kind: "task"; taskId: string }).taskId}
                activeTab={tab}
                hideDiff={isMobile()}
              />
            </Show>
          </div>
        </main>
      </div>
    </div>
  );
}
