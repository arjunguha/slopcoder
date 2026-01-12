import { createSignal, createResource, For, Show } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { listEnvironments, listBranches, createTask, createEnvironment } from "../api/client";
import type { AgentKind } from "../types";

const NEW_ENVIRONMENT_VALUE = "__new__";

export default function NewTaskForm() {
  const navigate = useNavigate();

  const [environments, { refetch: refetchEnvironments }] = createResource(listEnvironments);
  const [selectedEnv, setSelectedEnv] = createSignal("");
  const [newEnvName, setNewEnvName] = createSignal("");
  const [creatingEnv, setCreatingEnv] = createSignal(false);
  const [branches] = createResource(selectedEnv, async (env) => {
    if (!env || env === NEW_ENVIRONMENT_VALUE) return [];
    return listBranches(env);
  });

  const [baseBranch, setBaseBranch] = createSignal("");
  const [featureBranch, setFeatureBranch] = createSignal("");
  const [prompt, setPrompt] = createSignal("");
  const [agent, setAgent] = createSignal<AgentKind>("codex");
  const [submitting, setSubmitting] = createSignal(false);
  const [error, setError] = createSignal("");

  const isNewEnvironment = () => selectedEnv() === NEW_ENVIRONMENT_VALUE;

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    setError("");
    setSubmitting(true);

    try {
      let envName = selectedEnv();

      // If creating a new environment, create it first
      if (isNewEnvironment()) {
        const trimmedName = newEnvName().trim();
        if (!trimmedName) {
          setError("Please enter a name for the new environment");
          setSubmitting(false);
          return;
        }
        setCreatingEnv(true);
        try {
          const newEnv = await createEnvironment({ name: trimmedName });
          envName = newEnv.name;
          // Refetch environments to include the new one
          refetchEnvironments();
        } catch (err) {
          setError(err instanceof Error ? err.message : "Failed to create environment");
          setSubmitting(false);
          setCreatingEnv(false);
          return;
        }
        setCreatingEnv(false);
      }

      const result = await createTask({
        environment: envName,
        // For new environments, base branch defaults to "main" on the backend
        base_branch: isNewEnvironment() ? undefined : baseBranch() || undefined,
        feature_branch: featureBranch().trim() || undefined,
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
    if (isNewEnvironment()) {
      return newEnvName().trim() && prompt().trim();
    }
    return selectedEnv() && baseBranch().trim() && prompt().trim();
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
          {/* Environment */}
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
                onChange={(e) => {
                  setSelectedEnv(e.currentTarget.value);
                  setBaseBranch("");
                  setNewEnvName("");
                }}
                class={inputClass}
              >
                <option value="">Select</option>
                <option value={NEW_ENVIRONMENT_VALUE}>+ New Environment</option>
                <For each={environments()}>
                  {(env) => <option value={env.name}>{env.name}</option>}
                </For>
              </select>
            </Show>
          </div>

          {/* New Environment Name - shown when creating new environment */}
          <Show when={isNewEnvironment()}>
            <div>
              <label class="block text-xs font-medium text-gray-700 dark:text-gray-300 mb-1">
                Environment Name
              </label>
              <input
                type="text"
                value={newEnvName()}
                onInput={(e) => setNewEnvName(e.currentTarget.value)}
                placeholder="my-project"
                class={inputClass}
              />
            </div>
          </Show>

          {/* Base Branch - hidden when creating new environment */}
          <Show when={!isNewEnvironment()}>
            <div>
              <label class="block text-xs font-medium text-gray-700 dark:text-gray-300 mb-1">
                Base Branch
              </label>
              <Show
                when={selectedEnv()}
                fallback={
                  <div class="text-gray-400 dark:text-gray-500 text-xs">
                    Select environment
                  </div>
                }
              >
                <Show when={branches.loading}>
                  <div class="text-gray-500 dark:text-gray-400 text-xs">Loading branches...</div>
                </Show>
                <Show when={!branches.loading && !branches.error}>
                  <select
                    value={baseBranch()}
                    onChange={(e) => setBaseBranch(e.currentTarget.value)}
                    class={inputClass}
                  >
                    <option value="">Select</option>
                    <For each={branches()}>
                      {(b) => <option value={b}>{b}</option>}
                    </For>
                  </select>
                </Show>
                <Show when={branches.error}>
                  <div class="mt-1 text-xs text-red-600 dark:text-red-300">
                    Failed to load branches: {branches.error?.message}
                  </div>
                </Show>
              </Show>
            </div>
          </Show>

          {/* Feature Branch */}
          <div>
            <label class="block text-xs font-medium text-gray-700 dark:text-gray-300 mb-1">
              Feature Branch
            </label>
            <input
              type="text"
              value={featureBranch()}
              onInput={(e) => setFeatureBranch(e.currentTarget.value)}
              placeholder="Optional (auto-generate from prompt)"
              class={inputClass}
            />
          </div>
        </div>

        {/* Agent */}
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
        <div class="flex gap-3">
          <button
            type="submit"
            disabled={!isValid() || submitting()}
            class="flex-1 px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {creatingEnv() ? "Creating Environment..." : submitting() ? "Creating Task..." : "Create Task"}
          </button>
        </div>
      </form>
    </div>
  );
}
