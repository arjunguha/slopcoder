import type {
  Environment,
  BranchesResponse,
  Task,
  CreateTaskRequest,
  CreateTaskResponse,
  SendPromptRequest,
  CodexEvent,
  TaskOutputResponse,
  TaskDiffResponse,
} from "../types";

// Use relative URLs so the app works from any host
const API_BASE = import.meta.env.VITE_API_URL || "";

async function fetchJson<T>(url: string, options?: RequestInit): Promise<T> {
  const response = await fetch(`${API_BASE}${url}`, {
    ...options,
    headers: {
      "Content-Type": "application/json",
      ...options?.headers,
    },
  });

  if (!response.ok) {
    const error = await response.json().catch(() => ({ error: "Unknown error" }));
    throw new Error(error.error || `HTTP ${response.status}`);
  }

  return response.json();
}

// Environment endpoints
export async function listEnvironments(): Promise<Environment[]> {
  return fetchJson("/api/environments");
}

export async function listBranches(envName: string): Promise<string[]> {
  const data = await fetchJson<BranchesResponse>(
    `/api/environments/${encodeURIComponent(envName)}/branches`
  );
  return data.branches;
}

// Task endpoints
export async function listTasks(): Promise<Task[]> {
  return fetchJson("/api/tasks");
}

export async function getTask(id: string): Promise<Task> {
  return fetchJson(`/api/tasks/${id}`);
}

export async function createTask(req: CreateTaskRequest): Promise<CreateTaskResponse> {
  return fetchJson("/api/tasks", {
    method: "POST",
    body: JSON.stringify(req),
  });
}

export async function sendPrompt(taskId: string, req: SendPromptRequest): Promise<void> {
  await fetchJson(`/api/tasks/${taskId}/prompt`, {
    method: "POST",
    body: JSON.stringify(req),
  });
}

export async function getTaskOutput(taskId: string): Promise<TaskOutputResponse> {
  return fetchJson(`/api/tasks/${taskId}/output`);
}

export async function getTaskDiff(taskId: string): Promise<TaskDiffResponse> {
  return fetchJson(`/api/tasks/${taskId}/diff`);
}

// WebSocket for streaming events
export function subscribeToTask(
  taskId: string,
  onEvent: (event: CodexEvent) => void,
  onClose?: () => void
): () => void {
  // Build WebSocket URL from current location
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  const wsUrl = `${protocol}//${window.location.host}`;
  const ws = new WebSocket(`${wsUrl}/api/tasks/${taskId}/stream`);

  ws.onmessage = (event) => {
    try {
      const data = JSON.parse(event.data) as CodexEvent;
      onEvent(data);
    } catch (e) {
      console.error("Failed to parse event:", e);
    }
  };

  ws.onclose = () => {
    onClose?.();
  };

  ws.onerror = (error) => {
    console.error("WebSocket error:", error);
  };

  // Return cleanup function
  return () => {
    ws.close();
  };
}
