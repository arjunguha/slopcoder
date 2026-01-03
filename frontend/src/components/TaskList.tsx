import { createResource, For, Show } from "solid-js";
import { A } from "@solidjs/router";
import { listTasks } from "../api/client";
import type { Task } from "../types";

function StatusBadge(props: { status: Task["status"] }) {
  const colors = {
    pending: "bg-gray-500",
    running: "bg-blue-500 animate-pulse",
    completed: "bg-green-500",
    failed: "bg-red-500",
  };

  return (
    <span
      class={`inline-block px-2 py-1 text-xs font-medium text-white rounded ${colors[props.status]}`}
    >
      {props.status}
    </span>
  );
}

function TaskCard(props: { task: Task }) {
  const lastPrompt = () =>
    props.task.history.length > 0
      ? props.task.history[props.task.history.length - 1].prompt
      : "No prompts yet";

  return (
    <A
      href={`/tasks/${props.task.id}`}
      class="block p-4 bg-gray-100 dark:bg-gray-800 rounded-lg shadow hover:shadow-md transition-all border border-gray-200 dark:border-gray-700"
    >
      <div class="flex items-center justify-between mb-2">
        <h3 class="text-lg font-semibold text-gray-900 dark:text-gray-100">{props.task.name}</h3>
        <StatusBadge status={props.task.status} />
      </div>
      <div class="text-sm text-gray-600 dark:text-gray-400 mb-2">
        <span class="font-medium text-gray-700 dark:text-gray-300">{props.task.environment}</span>
        <span class="mx-2">•</span>
        <span>{props.task.branch}</span>
      </div>
      <p class="text-sm text-gray-500 truncate">{lastPrompt()}</p>
      <div class="mt-2 text-xs text-gray-400 dark:text-gray-500">
        {props.task.history.length} prompt{props.task.history.length !== 1 ? "s" : ""}
      </div>
    </A>
  );
}

export default function TaskList() {
  const [tasks, { refetch }] = createResource(listTasks);

  // Refetch every 5 seconds if any task is running
  setInterval(() => {
    const t = tasks();
    if (t?.some((task) => task.status === "running")) {
      refetch();
    }
  }, 5000);

  return (
    <div>
      <div class="flex items-center justify-between mb-6">
        <h1 class="text-2xl font-bold text-gray-900 dark:text-gray-100">Tasks</h1>
        <A
          href="/new"
          class="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors"
        >
          New Task
        </A>
      </div>

      <Show when={tasks.loading}>
        <div class="text-gray-500 dark:text-gray-400">Loading tasks...</div>
      </Show>

      <Show when={tasks.error}>
        <div class="p-4 bg-red-100 dark:bg-red-900/50 text-red-700 dark:text-red-200 rounded-lg border border-red-300 dark:border-red-700">
          Error loading tasks: {tasks.error?.message}
        </div>
      </Show>

      <Show when={tasks() && tasks()!.length === 0}>
        <div class="text-center py-12 text-gray-500 dark:text-gray-400">
          <p class="mb-4">No tasks yet</p>
          <A href="/new" class="text-blue-600 dark:text-blue-400 hover:underline">
            Create your first task
          </A>
        </div>
      </Show>

      <div class="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
        <For each={tasks()}>{(task) => <TaskCard task={task} />}</For>
      </div>
    </div>
  );
}
