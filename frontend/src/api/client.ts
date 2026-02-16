import type {
  Host,
  Environment,
  BranchesResponse,
  Task,
  CreateTaskRequest,
  CreateTaskResponse,
  CreateEnvironmentRequest,
  SendPromptRequest,
  AgentEvent,
  TaskOutputResponse,
  TaskDiffResponse,
} from "../types";

// Use relative URLs so the app works from any host
const API_BASE = import.meta.env.VITE_API_URL || "";
const PASSWORD_STORAGE_KEY = "slopcoderPassword";
let cachedPassword: string | null =
  typeof window === "undefined" ? null : window.localStorage.getItem(PASSWORD_STORAGE_KEY);

function setStoredPassword(password: string) {
  cachedPassword = password;
  window.localStorage.setItem(PASSWORD_STORAGE_KEY, password);
}

function clearStoredPassword() {
  cachedPassword = null;
  window.localStorage.removeItem(PASSWORD_STORAGE_KEY);
}

function promptForPassword(): string | null {
  if (typeof window === "undefined") {
    return null;
  }
  const password = window.prompt("Enter Slopcoder password:");
  if (password === null) {
    return null;
  }
  setStoredPassword(password);
  return password;
}

function buildHeaders(options?: RequestInit): HeadersInit {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (options?.headers) {
    Object.assign(headers, options.headers as HeadersInit);
  }
  if (cachedPassword) {
    headers["X-Slopcoder-Password"] = cachedPassword;
  }
  return headers;
}

async function fetchJson<T>(url: string, options?: RequestInit, retry = true): Promise<T> {
  const response = await fetch(`${API_BASE}${url}`, {
    ...options,
    headers: buildHeaders(options),
  });

  if (response.status === 401 && retry) {
    clearStoredPassword();
    const password = promptForPassword();
    if (password) {
      return fetchJson<T>(url, options, false);
    }
  }

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

export async function listHosts(): Promise<Host[]> {
  return fetchJson("/api/hosts");
}

export async function createEnvironment(req: CreateEnvironmentRequest): Promise<Environment> {
  return fetchJson("/api/environments", {
    method: "POST",
    body: JSON.stringify(req),
  });
}

export async function listBranches(envName: string, host?: string): Promise<string[]> {
  const query = host ? `?host=${encodeURIComponent(host)}` : "";
  const data = await fetchJson<BranchesResponse>(
    `/api/environments/${encodeURIComponent(envName)}/branches${query}`
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

export async function mergeTask(taskId: string): Promise<{ status: string; message: string }> {
  return fetchJson(`/api/tasks/${taskId}/merge`, {
    method: "POST",
  });
}

export async function getMergeStatus(
  taskId: string
): Promise<{ can_merge: boolean; reason: string | null }> {
  return fetchJson(`/api/tasks/${taskId}/merge-status`);
}

export async function archiveTask(taskId: string): Promise<{ status: string; message: string }> {
  return fetchJson(`/api/tasks/${taskId}/archive`, {
    method: "POST",
  });
}

export async function deleteTask(
  taskId: string,
  force = false
): Promise<{ status: string; message: string }> {
  const query = force ? "?force=true" : "";
  return fetchJson(`/api/tasks/${taskId}${query}`, {
    method: "DELETE",
  });
}

// WebSocket for streaming events
export function subscribeToTask(
  taskId: string,
  onEvent: (event: AgentEvent) => void,
  onClose?: () => void
): () => void {
  // Build WebSocket URL from current location
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  const wsUrl = `${protocol}//${window.location.host}`;
  const passwordQuery = cachedPassword ? `?password=${encodeURIComponent(cachedPassword)}` : "";
  const ws = new WebSocket(`${wsUrl}/api/tasks/${taskId}/stream${passwordQuery}`);
  let closedByClient = false;

  ws.onmessage = (event) => {
    try {
      const data = JSON.parse(event.data) as AgentEvent;
      onEvent(data);
    } catch (e) {
      console.error("Failed to parse event:", e);
    }
  };

  ws.onclose = () => {
    if (closedByClient) {
      return;
    }
    onClose?.();
  };

  ws.onerror = (error) => {
    console.error("WebSocket error:", error);
  };

  // Return cleanup function
  return () => {
    closedByClient = true;
    ws.close();
  };
}

export interface TerminalSession {
  sendInput: (data: Uint8Array) => void;
  resize: (rows: number, cols: number) => void;
  close: () => void;
}

export function subscribeToTerminal(
  taskId: string,
  onData: (data: Uint8Array) => void,
  onClose?: () => void
): TerminalSession {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  const wsUrl = `${protocol}//${window.location.host}`;
  const passwordQuery = cachedPassword ? `?password=${encodeURIComponent(cachedPassword)}` : "";
  const ws = new WebSocket(`${wsUrl}/api/tasks/${taskId}/terminal${passwordQuery}`);
  ws.binaryType = "arraybuffer";
  let closedByClient = false;

  ws.onmessage = (event) => {
    if (event.data instanceof ArrayBuffer) {
      onData(new Uint8Array(event.data));
      return;
    }
    if (event.data instanceof Blob) {
      void event.data.arrayBuffer().then((buffer) => onData(new Uint8Array(buffer)));
    }
  };

  ws.onclose = () => {
    if (!closedByClient) {
      onClose?.();
    }
  };

  ws.onerror = (error) => {
    console.error("Terminal WebSocket error:", error);
  };

  return {
    sendInput(data: Uint8Array) {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(data);
      }
    },
    resize(rows: number, cols: number) {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: "resize", rows, cols }));
      }
    },
    close() {
      closedByClient = true;
      ws.close();
    },
  };
}
