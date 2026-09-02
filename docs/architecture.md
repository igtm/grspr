# Architecture

## Product and review loop

`grspr` optimizes for one job: understanding a pull request without repeatedly
leaving the terminal. A review should feel like this:

```text
[choose base...head]
         |
         v
[changed files] <--> [viewport diff] --[ or ] key--> [adjacent hunk]
         |                   |
         |                   +--[p]--> [temporary file preview] --[Esc]--+
         |                                                            |
         +--[Ctrl+P / Ctrl+F]--> [repository result] --> [preview] ----+
         |
         +--[r]--> [persistent review state]
         |
         +--[w]--> [another worktree context]
```

The repository is never modified by review operations. Preview is temporary,
navigation preserves the review context, and every binding is discoverable from
the footer or help overlay.

The UI owns one mutable `App` state. Rendering is pure, while Git and ripgrep work runs in cancellable background tasks.

```text
[Crossterm events] ---> [App update] ---> [Ratatui render]
                            |
                            v
                         effects
                            |
                +-----------+-----------+
                |                       |
          [Git CLI task]          [ripgrep task]
                |                       |
                +------ bounded channel-+
                            |
                            v
                       [App update]
```

## Boundaries

- `git.rs` owns process invocation and repository semantics.
- `diff.rs` owns machine-output and unified-diff parsing.
- `review.rs` owns snapshot-scoped persistence.
- `worktree.rs` owns porcelain parsing.
- `app.rs` owns navigation, cancellation, and state transitions.
- `ui.rs` only turns current state into terminal cells.

Git is authoritative in the MVP so configured diff behavior and `base...head` semantics match the user's installation. Commands never pass through a shell, paths follow `--`, pagers and colors are disabled, and read operations set `GIT_OPTIONAL_LOCKS=0`.

Each diff or search request has a generation number and a cancellation token. Results from superseded generations are discarded. Diff stdout and ripgrep output are consumed line by line instead of first becoming one large `String`.

## Module map

```text
main.rs
  |
  +--> cli.rs
  +--> app.rs --------> ui.rs
         |
         +--> git.rs --> git / rg processes
         +--> diff.rs
         +--> review.rs
         +--> worktree.rs
```

`GitCli` is the single process boundary for the MVP. Application and rendering
code do not construct commands. If an in-process implementation becomes useful,
this concrete facade can become an injected backend without changing domain
types; adding one trait per command up front would not improve the current code.

## Technology choices

| Concern | Choice | Reason |
| --- | --- | --- |
| TUI | `ratatui` | Mature immediate-mode widgets and testable buffers |
| Terminal events | `crossterm` | Portable keyboard, mouse, resize, and async event stream |
| Git | Git CLI, not `git2` | Exact user Git semantics, worktree support, and less custom plumbing |
| Text search | `rg` subprocess | Best streaming monorepo search; cancellation kills the child |
| File enumeration | `git ls-files` | One required dependency and repository-aware ignored-file behavior |
| Fuzzy match | `nucleo-matcher` | Fast in-process matching with no process per keystroke |
| Highlighting | `syntect` | `bat`-class syntax definitions without a process per viewport update |
| Async work | `tokio` + channels | Keeps input/rendering responsive and makes stale work discardable |

Using `bat`, `fd`, or `delta` remains possible behind the process boundary, but
they are not required for the hot navigation loop.

## MVP boundary

Phase 1 includes repository/range detection, changed-file metadata, streamed
unified diffs, syntax highlighting, file/hunk navigation, previews, fuzzy file
lookup, streaming text search, persistent review marks, worktree listing/context
switching, and keyboard/basic mouse input.

Side-by-side and word-level diff, a searchable command palette, GitHub PR/API
integration and comments, worktree creation, dirty-state polling, symbols/LSP,
blame, and hunk-level review state are intentionally deferred. The current data
flow leaves room for them without placing GitHub or a parser in the render loop.
