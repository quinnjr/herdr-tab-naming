# herdr-tab-naming

A [Herdr](https://herdr.dev/) plugin that keeps every tab label in sync with
the repository and ticket the tab is actually working in.

```
1  ->  llama-rs
2  ->  llama-rs BACK-565
3  ->  advita-client-portal
```

Written in Rust. No Node, no per-event process storm, and it notices a plain
`cd`.

## Why not the usual "directory name" tab namer

`cd src/backend` should not rename a tab to `backend`, a linked worktree should
not lose its project name, and a shell `cd` should not wait for an unrelated
agent event to be noticed. This plugin:

- resolves the **git repository root** (worktree-aware), so `{repo}` is stable
  from any subdirectory and stays the main repo name inside `.worktrees/…`;
- runs as a **single resident watcher** instead of spawning a process per
  event;
- reconciles on a timer as well as on events, because Herdr emits no
  working-directory event — that timer is what catches a bare `cd`;
- records which labels it wrote, so a hand-renamed tab is left alone (unless
  you ask it not to).

## Install

Requires Herdr 0.8.0 or newer. From the marketplace / GitHub (builds on
install):

```bash
herdr plugin install quinnjr/herdr-tab-naming
```

For local development, build, copy the binary next to the manifest, and link
the checkout:

```bash
cargo build --release --locked
cp target/release/herdr-tab-naming .
herdr plugin link "$PWD"
```

The `startup` hook launches the watcher; the first reconcile adopts every tab,
so the existing labels are replaced once. Run the `Tab Naming: status` action
to see what will happen before it happens.

## Labels

Each tab is labelled from the working directory of its pane
(`foreground_cwd`, falling back to `cwd`):

1. Walk up to the nearest `.git` (directory or worktree/submodule file) and
   derive the repository.
2. Read `HEAD` directly — no `git` subprocess — for the branch.
3. Render the template, collapse whitespace, optionally truncate.
4. Fall back to the directory name outside a repository, or `~` at `$HOME`.

### Template tokens

| Token        | Meaning                                                    |
|--------------|------------------------------------------------------------|
| `{repo}`     | Main repository directory name (`llama-rs`)                |
| `{dir}`      | Working-directory name                                     |
| `{branch}`   | Branch name (`feature/BACK-565-thing`)                     |
| `{ticket}`   | `PREFIX-NUMBER` extracted from the branch (`BACK-565`)     |
| `{worktree}` | Linked worktree name, when the checkout is one             |
| `{agent}`    | Agent detected in the pane (`opencode`), when any          |
| `{status}`   | Agent status (`working`, `blocked`, …), when any           |

Default: `{repo} {ticket}`. On `develop` that is `llama-rs`; on
`feature/BACK-565-thing` it is `llama-rs BACK-565`. Unknown tokens are dropped.

## Configuration

Optional `config.json` in the plugin config dir (path printed by `status`);
defaults apply when it is absent:

```json
{
  "template": "{repo} {ticket}",
  "max_length": 0,
  "overwrite_manual": false,
  "poll_ms": 1000,
  "debounce_ms": 150,
  "ticket_prefixes": []
}
```

- `max_length` — truncate with `…`; `0` disables.
- `overwrite_manual` — when `true`, hand-renamed tabs are re-adopted every
  pass.
- `poll_ms` — reconcile interval, and the upper bound on `cd` latency.
- `ticket_prefixes` — restrict ticket extraction to these prefixes, e.g.
  `["BACK", "LIT"]`; empty accepts any `PREFIX-NUMBER`.

## Actions

| Action           | Effect                                             |
|------------------|----------------------------------------------------|
| `Tab Naming: sync now`        | Reconcile once, respecting manual names |
| `Tab Naming: re-adopt all tabs` | Reconcile and overwrite manual names  |
| `Tab Naming: status`          | Print current vs desired, no changes    |
| `Tab Naming: restart watcher` | Stop and start the watcher              |
| `Tab Naming: stop watcher`    | Ask the watcher to exit                 |

## How it works

- **Watcher** (`watch`): subscribes to Herdr's socket for structural events and
  reconciles on a timer. Renames only tabs whose computed label changed.
- **Ownership**: `labels.json` records the label written per tab. A label that
  differs from the record is treated as a manual rename and left alone.
- **Liveness**: manifest event hooks call `event`, which restarts the watcher
  if it died, so a crash self-heals on the next tab/pane activity.
- **State/logs** live in the plugin state dir (`watcher.log`, `labels.json`).

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
./herdr-tab-naming status   # needs HERDR_SOCKET_PATH in the environment
```
