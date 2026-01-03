import { createSignal, createResource, For, Show } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { listEnvironments, listBranches, createTask } from "../api/client";

export default function NewTaskForm() {
  const navigate = useNavigate();

  const [environments] = createResource(listEnvironments);
  const [selectedEnv, setSelectedEnv] = createSignal("");
  const [branches] = createResource(selectedEnv, async (env) => {
    if (!env) return [];
    return listBranches(env);
  });

  const [baseBranch, setBaseBranch] = createSignal("");
  const [featureBranch, setFeatureBranch] = createSignal("");
  const [prompt, setPrompt] = createSignal("");
  const [submitting, setSubmitting] = createSignal(false);
  const [error, setError] = createSignal("");

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    setError("");
    setSubmitting(true);

    try {
      const result = await createTask({
        environment: selectedEnv(),
        base_branch: baseBranch(),
        feature_branch: featureBranch(),
        prompt: prompt(),
      });
      navigate(`/tasks/${result.id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create task");
    } finally {
      setSubmitting(false);
    }
  };

  const isValid = () =>
    selectedEnv() && baseBranch().trim() && featureBranch().trim() && prompt().trim();

  const inputClass = "w-full px-4 py-2 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100 border border-gray-300 dark:border-gray-600 rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-transparent placeholder-gray-400 dark:placeholder-gray-500";

  return (
    <div class="max-w-2xl">
      <Show when={error()}>
        <div class="mb-4 p-4 bg-red-100 dark:bg-red-900/50 text-red-700 dark:text-red-200 rounded-lg border border-red-300 dark:border-red-700">
          {error()}
        </div>
      </Show>

      <form onSubmit={handleSubmit} class="space-y-6">
        {/* Environment */}
        <div>
          <label class="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
            Environment
          </label>
          <Show
            when={!environments.loading}
            fallback={<div class="text-gray-500 dark:text-gray-400">Loading environments...</div>}
          >
              <select
                value={selectedEnv()}
                onChange={(e) => {
                  setSelectedEnv(e.currentTarget.value);
                  setBaseBranch("");
                }}
                class={inputClass}
              >
              <option value="">Select an environment</option>
              <For each={environments()}>
                {(env) => <option value={env.name}>{env.name}</option>}
              </For>
            </select>
          </Show>
        </div>

        {/* Base Branch */}
        <div>
          <label class="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
            Base Branch
          </label>
          <Show
            when={selectedEnv()}
            fallback={
              <div class="text-gray-400 dark:text-gray-500 text-sm">
                Select an environment first
              </div>
            }
          >
            <Show
              when={!branches.loading}
              fallback={<div class="text-gray-500 dark:text-gray-400">Loading branches...</div>}
            >
              <select
                value={baseBranch()}
                onChange={(e) => setBaseBranch(e.currentTarget.value)}
                class={inputClass}
              >
                <option value="">Select a branch</option>
                <For each={branches()}>
                  {(b) => <option value={b}>{b}</option>}
                </For>
              </select>
            </Show>
          </Show>
        </div>

        {/* Feature Branch */}
        <div>
          <label class="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
            Feature Branch
          </label>
          <input
            type="text"
            value={featureBranch()}
            onInput={(e) => setFeatureBranch(e.currentTarget.value)}
            placeholder="e.g., feature/add-auth"
            class={inputClass}
          />
        </div>

        {/* Initial Prompt */}
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

        {/* Submit */}
        <div class="flex gap-4">
          <button
            type="submit"
            disabled={!isValid() || submitting()}
            class="flex-1 px-6 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {submitting() ? "Creating..." : "Create Task"}
          </button>
        </div>
      </form>
    </div>
  );
}
