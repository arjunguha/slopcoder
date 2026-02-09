# Slopcoder Design

## 1. Purpose and Scope

Slopcoder is a coordinator/agent web application for running coding agents against Git repositories using Git worktrees.

It provides:
- A Rust coordinator backend that serves the web UI/API and routes work to connected agents.
- A Rust `slopagent` worker process that owns local environments, tasks, persistence, and agent execution on each host.
- A SolidJS frontend for creating tasks, monitoring execution, sending follow-up prompts, reviewing diffs, and merging work.
- A core Rust library (`slopcoder-core`) that isolates environment, task, persistence, branch naming, and agent integration logic so behavior is testable independently from HTTP and UI layers.

The system is intentionally designed for trusted/self-hosted operation rather than multi-tenant security hardening.

## 2. High-Level Architecture

### 2.1 Components

1. `slopcoder-core` (`crates/slopcoder-core`)
- Environment config + git worktree operations.
- Task lifecycle state machine.
- YAML persistence (`tasks.yaml` per environment).
- Unified agent abstraction across Codex/Claude/Cursor/OpenCode/Gemini.
- Agent event parsing from heterogeneous JSON stream formats.
- Feature-branch auto-generation via DSRS (`dspy-rs`).

2. `slopcoder-server` (`crates/slopcoder-server`)
- Warp-based coordinator HTTP + WebSocket server.
- Tracks connected agents and host metadata.
- Aggregates environments/tasks across hosts and forwards task/environment RPC calls to the selected agent.
- Relays live task events from agents to browser WebSocket subscribers.
- Password auth for both browser API calls and agent connections.

3. `slopagent` (`crates/slopagent`)
- Runs on each managed host (including localhost if desired).
- Loads per-host `environments.yaml`.
- Owns local task lifecycle, persistence, branch creation, merge/diff/output operations.
- Connects outbound to coordinator `/agent/connect` and serves typed RPC requests.
- Streams task events back to coordinator.

3. Frontend (`frontend`)
- SolidJS single-page app with routes for task list/new task/task detail.
- API client for REST + WebSocket subscription.
- UI for environment/task creation, run monitoring, prompt continuation, diff inspection, and merge action.
- Rich event rendering for tool calls and agent messages.

### 2.2 Deployment Model

- Backend and frontend are served from the coordinator process (`slopcoder-server`):
  - `/api/*` routes for JSON APIs and task WebSocket streams.
  - `/agent/connect` WebSocket endpoint for connected `slopagent` workers.
  - Static frontend assets from `frontend/dist` (or overridden static dir).
- Each `slopagent` uses file-based per-host configuration (`environments.yaml`).
- Persistent task metadata remains file-based per environment (`tasks.yaml` and `task-<id>.jsonl`), written by the owning agent host.

## 3. Core Domain Model

### 3.1 Environment

Implemented in `crates/slopcoder-core/src/environment.rs`.

An environment contains:
- `name`: logical identifier shown in UI.
- `directory`: filesystem root containing:
  - `bare/` bare Git repository.
  - worktree directories created by branch name.
  - `tasks.yaml` and task output logs.

`EnvironmentConfig` includes:
- `new_environments_directory`: required directory where new environments can be initialized.
- `environments`: list of configured environments.

Key operations:
- Validate environment structure (`bare/HEAD` exists).
- List branches from bare repo.
- Create worktree for existing branch.
- Create feature worktree from base branch (`git worktree add -b ...`).
- Initialize brand-new environment (create bare repo, seed initial commit, append to config).

### 3.2 Task

Implemented in `crates/slopcoder-core/src/task.rs`.

A task represents one agent-backed branch/worktree session:
- `id` (`UUID`).
- `agent` (`codex`, `claude`, `cursor`, `opencode`, `gemini`).
- `environment`, `base_branch`, `feature_branch`, `worktree_path`.
- `status`: `pending | running | completed | failed | interrupted`.
- `session_id` for follow-up prompts.
- `history`: list of prompt runs with timestamps and success values.

State transitions:
- `pending/completed/failed/interrupted -> running` on new prompt.
- `running -> completed|failed` on normal completion.
- `running -> interrupted` on user interrupt.

## 4. Persistence Design

Implemented in `crates/slopcoder-core/src/persistence.rs`.

### 4.1 Data placement

Per environment directory:
- `tasks.yaml`: serialized task metadata.
- `task-<task_id>.jsonl`: append-only normalized event log used for API replay.

### 4.2 PersistentTaskStore behavior

- Environments are explicitly registered in memory with name -> directory mapping.
- `load_all()` reads each environment's `tasks.yaml`, then:
  - removes tasks whose worktrees no longer exist.
  - marks tasks stuck in `running` as `failed` (crash recovery).
  - writes back if cleanup/recovery changed data.
- Writes are environment-scoped: changing one task rewrites that environment's `tasks.yaml` snapshot.

### 4.3 Stale worktree cleanup

To handle external CLI operations (e.g., manual `git worktree remove`), stale tasks are cleaned both:
- At startup (via load/validation).
- During task listing (`cleanup_stale_tasks()`), keeping UI and YAML aligned with filesystem truth.

## 5. Agent Abstraction and Execution

### 5.1 Unified interface

Implemented in `crates/slopcoder-core/src/anyagent.rs`.

`AnyAgent` trait provides:
- `next_event()` streaming event pull.
- `wait()` completion result.
- `kill()` interrupt support.
- `session_id()` extraction for resume.

### 5.2 Concrete adapters

- `codex_agent.rs`: `codex exec --json --dangerously-bypass-approvals-and-sandbox`.
- `claude_agent.rs`: `claude --print --output-format stream-json --dangerously-skip-permissions`.
- `cursor_agent.rs`: `cursor-agent --print --output-format stream-json --force`.
- `gemini_agent.rs`: `gemini --output-format stream-json --approval-mode yolo`.
- `opencode_agent.rs`: `opencode run --format json ...`; stores UUID<->native session string mapping in `.opencode-sessions.json` for resume semantics.

### 5.3 Event normalization

Implemented in `crates/slopcoder-core/src/events.rs`.

Different CLI stream formats are parsed and normalized into `AgentEvent` variants:
- `session.started`
- `turn.started` / `turn.completed`
- `item.completed` (reasoning, messages, tool call/output)
- `background_event`
- `prompt.sent`

Notable design choice:
- Unknown/extra fields are tolerated to avoid breakage on upstream schema drift.
- Serialization for flattened `extra` handles non-object values safely.

## 6. Branch Name Generation

Implemented in `crates/slopcoder-core/src/branch_picker.rs`.

When feature branch is omitted in task creation:
- Server calls `pick_feature_branch(prompt, model)`.
- Uses `dspy-rs` with OpenAI-compatible model endpoint.
- Requires `OPENAI_API_KEY`.
- Normalization constrains branch names (sanitized characters, max length 40).
- On generation failure, API returns explicit validation error requiring manual branch input.

## 7. Server State and API Design

### 7.1 Coordinator AppState

Implemented in `crates/slopcoder-server/src/state.rs`.

State owns:
- Connected agent registry (host label, hostname, connection metadata).
- Task-to-host routing map for RPC forwarding.
- Per-task broadcast channels for browser WebSocket streaming.
- Optional auth password.

Startup behavior:
- Configure optional/random password.
- Start API/UI server and accept dynamic agent connections.

### 7.2 Routes

Implemented in `crates/slopcoder-server/src/routes.rs`.

Environment routes:
- `GET /api/environments`
- `POST /api/environments` (initialize new env)
- `GET /api/environments/{name}/branches?host=...`
- `GET /api/hosts`

Task routes:
- `GET /api/tasks`
- `POST /api/tasks`
- `GET /api/tasks/{id}`
- `POST /api/tasks/{id}/prompt`
- `GET /api/tasks/{id}/output`
- `GET /api/tasks/{id}/diff`
- `POST /api/tasks/{id}/interrupt`
- `POST /api/tasks/{id}/merge`
- `GET /api/tasks/{id}/stream` (WebSocket)
- `GET /agent/connect` (WebSocket; slopagent connection endpoint)

Routing behavior:
- API rejection recovery is scoped under `/api/*`, so non-API paths continue to static file and SPA fallback handlers instead of returning API JSON 404 payloads.
- `500` API responses are logged with contextual server-side error messages, and unknown Warp rejections are logged before returning a generic JSON internal error to clients.
- Auth query parsing treats missing query strings as empty input (instead of rejecting requests), preventing spurious `InvalidQuery` rejections on normal API calls without URL query parameters.

### 7.3 RPC and task orchestration

- Coordinator sends typed `AgentRequest` RPC envelopes to the selected host.
- Agent responds with typed `AgentResponse` (or structured error status/message).
- Running tasks emit `TaskEvent` envelopes from agent to coordinator.
- Coordinator rebroadcasts those events to browser `/api/tasks/{id}/stream` subscribers.
- All task execution/persistence/diff/merge logic executes on the agent host.

### 7.4 Diff and merge

Diff endpoint:
- Returns staged and unstaged patches separately.
- Includes untracked files in unstaged via `/dev/null` diff.

Merge endpoint:
- Rejects when source task worktree or target base worktree is dirty (staged/unstaged/untracked).
- Ensures target base worktree exists (creates if absent).
- Executes `git merge <feature_branch>` in target worktree.
- On conflict/failure, aborts merge and returns conflict response.

### 7.5 Authentication

Password mode:
- By default, coordinator authentication is disabled.
- Can be overridden with `--password`, prompted via `--password-prompt`, or disabled via `--no-password`.
- Required value is checked against:
  - `X-Slopcoder-Password` header (REST), or
  - `password` query parameter (browser task websocket).
- Agent websocket auth uses the same `X-Slopcoder-Password` header.

## 8. Frontend Design

### 8.1 Routing and layout

Implemented in `frontend/src/App.tsx`.

Routes:
- `/`, `/new`, `/tasks/:id`: all render a unified single-page workspace shell.

Workspace layout (implemented in `frontend/src/components/Workspace.tsx`):
- Left pane: minimal (unboxed) expandable environment tree with per-environment task list, collapse/expand chevrons, and `+` action per environment to launch a task-creation compose screen.
 - Right pane: context-driven content area that switches between:
   - task conversation/diff tabs for selected tasks,
   - new task compose screen for a selected environment,
   - new environment compose screen.
 - A single top-level `+ New` action is used to enter new-environment flow.

### 8.2 API client

Implemented in `frontend/src/api/client.ts`.

- Uses relative URL base by default for same-origin deployment.
- Centralized `fetchJson` handles JSON parsing and errors.
- Password caching in `localStorage`; 401 triggers password reprompt.
- WebSocket URL built from current host/protocol.

### 8.3 New Task flow

Implemented in `frontend/src/components/Workspace.tsx` (`NewTaskPane`).

- Triggered from per-environment `+` action in the left tree.
- Environment identity is `(host, environment)`.
- New Environment flow requires explicit host selection from connected agents.
- Base branch selectable from live branch list for that environment.
- Feature branch optional (auto-generated server-side if omitted).
- Agent selection exposed in UI: `codex`, `claude`, `cursor`, `gemini`, `opencode`.
- Prompt field is auto-focused when entering create-task mode.
- Selector controls (agent/base/feature) are placed directly above the prompt compose box.
- On submit: creates the task, then opens it in the right pane conversation tab.

### 8.4 Task list/detail

- `Workspace.tsx` owns host/environment/task fetching and periodic refresh, and renders task selection as a host-aware left tree instead of a separate list page.
- `TaskPane` within `Workspace.tsx` handles live stream + persisted output rendering, prompt continuation, status display, and merge action.
- Right-pane tab model splits task content into explicit `Conversation` and `Diff` tabs.
- `DiffViewer.tsx` remains the diff renderer for staged/unstaged changes.
- Markdown rendering uses explicit light/dark CSS for fenced and inline code blocks to keep code text/background contrast correct under system dark mode.

## 9. Evolution Highlights from Git History

Recent design-driven feature additions include:
- Multi-agent backend support (Cursor, OpenCode, Gemini).
- DSRS-based branch naming and stricter normalized branch constraints.
- Persistent per-environment YAML task storage with stale-worktree cleanup and crash recovery.
- New environment initialization API/UI path.
- Task diff improvements (staged/unstaged split + untracked file support).
- Merge-to-base workflow from task detail.
- Optional password-based API access.
- Frontend formatting improvements for tool-call readability and output summarization.

## 10. Testing and Quality Strategy

- Rust unit tests cover task lifecycle, persistence, environment operations, route helpers, merge behavior, and event parsing.
- Integration tests in `slopcoder-core/tests/integration.rs` validate environment/worktree workflows; optional feature-gated tests exercise real agent CLIs.
- Frontend has utility tests (`messageFormatting.test.ts`) and build/test scripts via npm.
- CI runs on self-hosted runner with Rust workspace tests and selected features.

## 11. Known Constraints and Tradeoffs

- Security model is intentionally minimal; deployment assumes trusted network context.
- Persistence rewrites full environment task snapshots (`tasks.yaml`) rather than append-only incremental updates.
- WebSocket streaming depends on in-memory channels for active runs; historical replay is via log file endpoint, not stream rewind.
- Agent availability is dynamic; disconnected hosts disappear from active UI/API results until they reconnect.
- OpenCode resume requires sidecar session map file because native session IDs are not UUIDs.
- Agent capability remains host-dependent (CLI binaries/tool auth must already be present on that host).
