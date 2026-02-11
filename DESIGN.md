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

`environments.yaml` schema:

```yaml
worktrees_directory: "/path/to/isolated/worktrees"
environments:
  - "/path/to/repo-a"
  - "/path/to/repo-b"
```

Semantics:
- `environments` is a list of repository directories.
- `worktrees_directory` is the parent directory used for isolated task worktrees.
- Environment IDs are the directory paths.

Validation:
- `worktrees_directory` must exist and be a directory.
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

Rationale:
- Keeps environment repositories clean (no metadata files showing up as untracked changes).
- Prevents merge checks from failing due to Slopcoder’s own files.

Store behavior:
- Load per-environment tasks from `tasks.yaml`.
- Remove tasks whose workspace directory no longer exists.
- Mark stale `running` tasks as `failed` on restart.
- Rewrite only affected environment task snapshots.

## 6. Agent-Side Task Creation and Merge

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
- Merge runs in environment repo directory: `git merge <merge_branch>`.
- Conflict path aborts merge and returns an error.

Diff behavior:
- For worktree tasks, staged diff is against `base_branch`.
- For in-place tasks, staged diff is regular cached diff in current repo state.
- Unstaged includes tracked + untracked changes.

## 7. Coordinator and API

Coordinator routes live in `crates/slopcoder-server/src/routes.rs`.

Task creation payload:
- `host`, `environment`, optional `name`, `use_worktree`, `prompt`, `agent`.

Task response payload now includes:
- `name`
- `workspace_kind`
- `base_branch` (optional)
- `merge_branch` (optional)

Environment creation via API is disabled in the agent:
- Environments are declared in `environments.yaml`.

## 8. Frontend Model

Primary UI is `frontend/src/components/Workspace.tsx`.

Behavior:
- Environment list maps directly to configured repository directory entries.
- New task form no longer asks for base/feature branch.
- User can toggle `Run task in isolated worktree (mergeable)`.
- Task list and task header display task `name` (topic).
- Merge controls are shown only for `worktree` tasks.

## 9. Testing

- Rust unit/integration tests cover environment operations, persistence behavior, and task lifecycle.
- Frontend build runs TypeScript typecheck and Vite build.
- End-to-end behavior remains host-local on `slopagent`, with coordinator acting as RPC relay.
