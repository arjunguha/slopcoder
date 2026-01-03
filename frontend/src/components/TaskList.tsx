import { createResource, For, Show } from "solid-js";
import { A } from "@solidjs/router";
import { listTasks } from "../api/client";
import NewTaskForm from "./NewTaskForm";
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
  return (
    <A
      href={`/tasks/${props.task.id}`}
      class="block p-4 bg-gray-100 dark:bg-gray-800 rounded-lg shadow hover:shadow-md transition-all border border-gray-200 dark:border-gray-700"
    >
      <div class="flex items-center justify-between mb-2">
        <h3 class="text-lg font-semibold text-gray-900 dark:text-gray-100">
          {props.task.environment}/{props.task.feature_branch}
        </h3>
        <StatusBadge status={props.task.status} />
      </div>
      <div class="text-xs text-gray-500 dark:text-gray-400">
        {props.task.agent} • worktree date: {props.task.worktree_date ?? "unknown"}
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
      <div class="mb-10">
        <h1 class="text-2xl font-bold text-gray-900 dark:text-gray-100 mb-4">New Task</h1>
        <div class="p-6 bg-gray-50 dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800">
          <NewTaskForm />
        </div>
      </div>

      <div class="flex items-center justify-between mb-6">
        <h2 class="text-xl font-bold text-gray-900 dark:text-gray-100">Tasks</h2>
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
        </div>
      </Show>

      <div class="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
        <For each={tasks()}>{(task) => <TaskCard task={task} />}</For>
      </div>
    </div>
  );
}
