// API Types

export interface Environment {
  name: string;
  directory: string;
}

export interface BranchesResponse {
  branches: string[];
}

export interface PromptRun {
  prompt: string;
  started_at: string;
  finished_at: string | null;
  success: boolean | null;
}

export interface Task {
  id: string;
  name: string;
  environment: string;
  branch: string;
  status: "pending" | "running" | "completed" | "failed";
  session_id: string | null;
  created_at: string;
  history: PromptRun[];
}

export interface CreateTaskRequest {
  name: string;
  environment: string;
  branch: string;
  prompt: string;
}

export interface CreateTaskResponse {
  id: string;
  worktree_path: string;
}

export interface SendPromptRequest {
  prompt: string;
}

// Codex Event Types (from WebSocket)

export interface CompletedItem {
  id: string;
  type: string;
  text?: string;
  name?: string;
  arguments?: string;
  call_id?: string;
  output?: string;
}

export interface UsageStats {
  input_tokens?: number;
  cached_input_tokens?: number;
  output_tokens?: number;
}

export type CodexEvent =
  | { type: "thread.started"; thread_id: string }
  | { type: "turn.started" }
  | { type: "item.completed"; item: CompletedItem }
  | { type: "turn.completed"; usage?: UsageStats }
  | { type: "background_event"; event?: string }
  | { type: "unknown" };
