# Slopcoder Design

## 1. Purpose

Slopcoder coordinates coding agents across one or more hosts, each host running `slopagent` against local checked-out Git repositories.

The system has four parts:
- `slopcoder-core`: shared domain logic (environments, tasks, persistence, agent adapters, naming).
- `slopagent`: host-local worker that owns task lifecycle and Git operations.
- `slopcoder-server`: coordinator API/UI server and RPC relay.
- `frontend`: SolidJS UI for creating/running/merging tasks.

## 2. Environment Model

Implemented in `crates/slopcoder-core/src/environment.rs`.

An environment is now a **checked-out Git repository directory** (not a bare repo root).
`slopagent` configuration is CLI-driven for discovery (`REPO_ROOT` + bounds), while
environment/worktree storage roots are fixed to the XDG data directory.

Semantics:
- CLI positional `REPO_ROOT` is required and is scanned for repositories.
- Storage root is `$XDG_DATA_HOME/slopcoder` (fallback: `~/.local/share/slopcoder`).
- Worktrees live under `<storage_root>/worktrees`.
- Create-environment and environment discovery root is `<storage_root>/environments`.
- Discovery is recursive, bounded (`max_depth` default `10`, `max_repos` default `100`), skips hidden directories,
  and does not descend into directories that are already Git repositories.
- `slopagent` serves environment lists from an in-memory cache and refreshes discovery in the background on a short interval,
  so repository scans do not block request handling.
- Environment IDs are the directory paths.

Validation:
- `worktrees_directory` is created at startup if missing, then validated as a directory.
- Each environment must satisfy `git rev-parse --is-inside-work-tree`.

Core operations:
- `list_branches()` from the checked-out repository.
- `current_branch()` for in-repo HEAD branch resolution.
- `create_worktree_from_base(worktrees_directory, base_branch, merge_branch)` for isolated tasks.

## 3. Task Model

Implemented in `crates/slopcoder-core/src/task.rs`.

Task fields:
- `id`, `agent`, `environment`, `name`, `worktree_path`, `status`, `session_id`, `created_at`, `history`.
- `workspace_kind`: `environment` or `worktree`.
- `base_branch` and `merge_branch` are set only for `worktree` tasks.
- `web_search`: task-level boolean persisted with the task and reused on prompt resumes.

Task behavior:
- Every task runs in exactly one directory (`worktree_path`).
- `environment` tasks run directly in the environment repo directory.
- `worktree` tasks run in a newly created isolated worktree and are mergeable.

State transitions:
- `pending/completed/failed/interrupted -> running`
- `running -> completed|failed|interrupted`

## 4. Task Naming (DSPy)

Implemented in `crates/slopcoder-core/src/branch_picker.rs`.

Task names are no longer feature branch names.

Flow:
- `pick_task_topic(prompt, model)` asks DSPy for a short topic (max 20 chars).
- On failure, fallback is the first 20 chars of the prompt (`fallback_topic_name`).

For isolated worktrees, merge branches are internal and generated from topic slug + random suffix:
- Example: `task/fix-login-flow-a1b2c3d4`.

## 5. Persistence

Implemented in `crates/slopcoder-core/src/persistence.rs` and `crates/slopagent/src/state.rs`.

Persistence is file-based, but now stored **outside** repository working directories:
- Root: `<worktrees_directory>/.slopcoder-state/<env-slug>/`
- Files:
  - `tasks.yaml`
  - `task-<task_id>.jsonl`
- Archive root: `<worktrees_directory>/.slopcoder-state/archive/<env-slug>/`
  - `task-<task_id>.jsonl` (moved here when archived or deleted)

Rationale:
- Keeps environment repositories clean (no metadata files showing up as untracked changes).
- Prevents merge checks from failing due to Slopcoder’s own files.

Store behavior:
- Load per-environment tasks from `tasks.yaml`.
- Remove tasks whose workspace directory no longer exists.
- Mark stale `running` tasks as `failed` on restart.
- Rewrite only affected environment task snapshots.

## 6. Agent-Side Task Creation, Merge, Archive, and Delete

Implemented in `crates/slopagent/src/main.rs`.

`create_task` modes:
- In-place (`use_worktree=false`):
  - `worktree_path = environment.directory`
  - No merge branch; task is not mergeable via UI API.
- Isolated (`use_worktree=true`):
  - Resolve `base_branch` from environment current branch.
  - Create new merge branch and worktree under `worktrees_directory`.
  - Task is mergeable.

Merge rules:
- Only `workspace_kind == worktree` tasks can be merged.
- Task worktree must be clean.
- Environment repo must be clean.
- Merge availability precheck uses `git merge-tree --write-tree HEAD <merge_branch>`.
- Merge runs in environment repo directory: `git merge <merge_branch>`.
- Conflict path aborts merge and returns an error.

Archive/delete rules:
- `archive` is for `environment` tasks: move `task-<id>.jsonl` to archive directory and remove task from active list.
- `delete` is for `worktree` tasks: prune the worktree, archive `task-<id>.jsonl`, remove task from active list, and attempt branch cleanup.
- Non-force prune may fail when modified/untracked files exist; API returns a conflict instructing force prune.

Diff behavior:
- For worktree tasks, staged diff is against `base_branch`.
- For in-place tasks, staged diff is regular cached diff in current repo state.
- Unstaged includes tracked + untracked changes.

## 7. Coordinator and API

Coordinator routes live in `crates/slopcoder-server/src/routes.rs`.

Coordinator request model:
- Multi-host fan-out endpoints (environment/task listing and task lookup fallback) query hosts in parallel instead of serially.
- Per-host coordinator RPC calls use bounded route-level timeouts to keep UI handlers responsive even when one host is slow.
- Timed-out/disconnected pending RPC entries are explicitly cleaned up in coordinator state.
- Agent RPC requests are handled concurrently per request ID, so a long-running request (for example, environment discovery)
  does not block unrelated agent operations on the same connection.

Task creation payload:
- `host`, `environment`, optional `name`, `use_worktree`, `web_search`, `prompt`, `agent`.

Task response payload now includes:
- `name`
- `workspace_kind`
- `base_branch` (optional)
- `merge_branch` (optional)

Task action endpoints:
- `POST /api/tasks/:id/merge`
- `GET /api/tasks/:id/merge-status` (returns `can_merge` + reason)
- `POST /api/tasks/:id/archive`
- `DELETE /api/tasks/:id?force=true|false`
- `GET /api/tasks/:id/terminal` (websocket PTY for interactive terminal I/O)

Environment creation via API:
- UI provides host + environment name only.
- Agent creates `<storage_root>/environments/<name>`, initializes a Git repository, and makes an empty initial commit.
- Created/discovered environments are listed immediately without local config-file writes.
- The create-environment screen also captures an initial prompt and immediately creates the first task in the new environment.

## 8. Frontend Model

Primary UI is `frontend/src/components/Workspace.tsx`.

Behavior:
- Environment list includes repositories auto-discovered under `environments_root` and optional `repo_root`.
- Environment list includes repositories auto-discovered under `<storage_root>/environments` and optional `repo_root`.
- "Create Environment" button appears above the list and opens a host+name form.
- The create-environment form uses the same centered "Let's Build" visual style and starts the first task immediately after environment creation.
- New task form no longer asks for base/feature branch.
- User can toggle `Run task in isolated worktree (mergeable)`.
- `Enable web search` is shown only when the selected agent supports it.
- `web_search` is currently wired to Codex (`--search`) and ignored for other agents.
- Prompt textareas in task-creation panes support `Ctrl+Enter` (and `Cmd+Enter`) to submit without clicking.
- Task conversation follow-up drafts are cached locally per task ID in browser `localStorage`; drafts are not persisted on the server.
- Task list and task header display task `name` (topic).
- `environment` tasks show an archive button beside the task title.
- `worktree` tasks show merge/delete actions (no archive button).
- Merge action uses server-side merge readiness and is disabled when merge cannot currently succeed.
- Delete action uses an inline warning dialog (no JS modal) and supports force prune when normal prune fails.
- Unused legacy `frontend/src/components/TaskDetail.tsx` has been removed; `Workspace.tsx` is the only task conversation UI.
- Task detail reactive resources must not reference `taskData` before it is initialized (prevents runtime TDZ errors when opening tasks).
- Opening a different task conversation now auto-scrolls to the bottom of the transcript.
- Switching back to the Conversation tab also auto-scrolls the transcript to the newest message.
- Conversation transcripts now render progressively in chunks (latest-first) to reduce UI lag on long histories.
- Scrolling to the top of the transcript incrementally reveals older events.
- Live conversation streaming avoids subscription churn during task polling to reduce update flicker.
- `command_execution` transcript items now render as command cards showing the command text and at most the first 5 lines of output (with truncation indicator).
- Task detail tabs now include `Terminal` beside `Conversation` and `Diff` on desktop.
- Terminal uses `xterm` over a coordinator websocket that proxies I/O to the owning `slopagent` host.
- Terminal starts in the selected task workspace directory on that remote host (`worktree_path` for isolated tasks, environment directory for in-place tasks).
- Terminal websocket supports dynamic PTY resize so the shell tracks pane/window dimensions.

## 9. Testing

- Rust unit/integration tests cover environment operations, persistence behavior, and task lifecycle.
- Frontend build runs TypeScript typecheck and Vite build.
- End-to-end behavior remains host-local on `slopagent`, with coordinator acting as RPC relay.
