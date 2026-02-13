import { createSignal, createResource, For, Show } from "solid-js";
import { useNavigate } from "@solidjs/router";
import {
  listEnvironments,
  createTask,
} from "../api/client";
import type { AgentKind } from "../types";

export default function NewTaskForm() {
  const navigate = useNavigate();

  const [environments] = createResource(listEnvironments);
  const [selectedEnv, setSelectedEnv] = createSignal("");
  const [taskName, setTaskName] = createSignal("");
  const [useWorktree, setUseWorktree] = createSignal(false);
  const [webSearch, setWebSearch] = createSignal(false);
  const [prompt, setPrompt] = createSignal("");
  const [agent, setAgent] = createSignal<AgentKind>("codex");
  const [submitting, setSubmitting] = createSignal(false);
  const [error, setError] = createSignal("");

  const selectedEnvParts = () => {
    const [host, env] = selectedEnv().split("::");
    if (!host || !env) return null;
    return { host, env };
  };

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    setError("");
    setSubmitting(true);

    try {
      const selected = selectedEnvParts();
      if (!selected) {
        setError("Please select an environment");
        setSubmitting(false);
        return;
      }

      const result = await createTask({
        host: selected.host,
        environment: selected.env,
        name: taskName().trim() || undefined,
        use_worktree: useWorktree(),
        web_search: webSearch(),
        prompt: prompt(),
        agent: agent(),
      });
      navigate(`/tasks/${result.id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create task");
    } finally {
      setSubmitting(false);
    }
  };

  const isValid = () => {
    return selectedEnvParts() && prompt().trim();
  };

  const inputClass = "w-full px-3 py-1.5 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100 border border-gray-300 dark:border-gray-600 rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-transparent placeholder-gray-400 dark:placeholder-gray-500";

  return (
    <div class="max-w-2xl">
      <Show when={error()}>
        <div class="mb-4 p-4 bg-red-100 dark:bg-red-900/50 text-red-700 dark:text-red-200 rounded-lg border border-red-300 dark:border-red-700">
          {error()}
        </div>
      </Show>

      <form onSubmit={handleSubmit} class="space-y-4">
        <div class="grid gap-3 md:grid-cols-3">
          <div>
            <label class="block text-xs font-medium text-gray-700 dark:text-gray-300 mb-1">
              Environment
            </label>
            <Show
              when={!environments.loading}
              fallback={<div class="text-gray-500 dark:text-gray-400 text-xs">Loading environments...</div>}
            >
              <select
                value={selectedEnv()}
                onChange={(e) => setSelectedEnv(e.currentTarget.value)}
                class={inputClass}
              >
                <option value="">Select</option>
                <For each={environments()}>
                  {(env) => <option value={`${env.host}::${env.name}`}>{env.host}/{env.name}</option>}
                </For>
              </select>
            </Show>
          </div>

          <div>
            <label class="block text-xs font-medium text-gray-700 dark:text-gray-300 mb-1">
              Task Topic
            </label>
            <input
              type="text"
              value={taskName()}
              onInput={(e) => setTaskName(e.currentTarget.value)}
              placeholder="Optional (auto-generate)"
              class={inputClass}
            />
          </div>

          <div>
            <label class="block text-xs font-medium text-gray-700 dark:text-gray-300 mb-1">
              Agent
            </label>
            <select
              value={agent()}
              onChange={(e) => setAgent(e.currentTarget.value as AgentKind)}
              class={inputClass}
            >
              <option value="codex">Codex</option>
              <option value="claude">Claude</option>
              <option value="cursor">Cursor</option>
              <option value="gemini">Gemini</option>
            </select>
          </div>
        </div>

        <label class="flex items-center gap-2 text-xs text-gray-700 dark:text-gray-300">
          <input
            type="checkbox"
            checked={useWorktree()}
            onChange={(e) => setUseWorktree(e.currentTarget.checked)}
          />
          Run in isolated worktree (mergeable)
        </label>
        <label class="flex items-center gap-2 text-xs text-gray-700 dark:text-gray-300">
          <input
            type="checkbox"
            checked={webSearch()}
            onChange={(e) => setWebSearch(e.currentTarget.checked)}
          />
          Enable web search (Codex)
        </label>

        <div>
          <label class="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
            Initial Prompt
          </label>
          <textarea
            value={prompt()}
            onInput={(e) => setPrompt(e.currentTarget.value)}
            placeholder="Describe what you want the agent to do..."
            rows={6}
            class={`${inputClass} resize-none`}
          />
        </div>

        <div class="flex gap-3">
          <button
            type="submit"
            disabled={!isValid() || submitting()}
            class="flex-1 px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {submitting() ? "Creating Task..." : "Create Task"}
          </button>
        </div>
      </form>
    </div>
  );
}
