# grspr

> Grasp the whole PR.

`grspr` is a fast, local-first terminal UI for human review of pull-request-sized Git diffs. It keeps changed files, hunks, repository search, file context, worktrees, and review progress in one keyboard-driven workspace.

<!-- Demo assets can be added here without restructuring the document.
![grspr demo](docs/assets/demo.gif)
-->

## Status

`grspr` is an early MVP. It is read-only with respect to the repository: marking a file reviewed only writes local application state.

Implemented:

- PR-style `base...head` revision ranges
- added, modified, deleted, renamed, and binary file metadata
- streamed, viewport-limited unified diff rendering
- syntax highlighting and old/new line numbers
- file and hunk navigation
- context-line and whitespace toggles
- full-file preview
- fuzzy repository file finder
- streaming ripgrep results with cancellation
- persistent Viewed/Reviewed state
- worktree list and context switching
- keyboard and basic mouse navigation
- an explicit empty state that distinguishes an existing PR, no PR, and unavailable PR lookup

Side-by-side diff, inline comments, GitHub API integration, and worktree creation are planned after the local review loop is hardened.

## Requirements

- Git
- ripgrep (`rg`) for repository text search
- GitHub CLI (`gh`) is optional and only used to identify PR status for an empty diff
- a terminal with 256-color or true-color support

## Install

From a GitHub release on Linux or macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/igtm/grspr/main/install.sh | sh
```

While the repository is private, use an authenticated GitHub CLI. The installer
also detects `gh` automatically when downloading private release assets.

```sh
gh api repos/igtm/grspr/contents/install.sh \
  -H 'Accept: application/vnd.github.raw+json' | sh
```

Choose a version or install directory:

```sh
./install.sh -v=0.0.2
./install.sh -b=/usr/local/bin
```

From source:

```sh
cargo install --path .
```

## Usage

```sh
grspr main...HEAD
grspr --base main --head feature/auth
grspr -C ../another-worktree main...HEAD
```

Without a base, `grspr` tries `origin/HEAD`, `main`, then `master`. If none exists, pass an explicit base.

## Essential keys

| Key | Action |
| --- | --- |
| `j` / `k` | Move in the focused panel |
| `J` / `K` | Next / previous changed file |
| `]` / `[` | Next / previous hunk |
| `Tab` | Switch Files/Diff focus |
| `p`, `Space`, `Enter` | Preview selected file |
| `r` | Toggle Reviewed |
| `Ctrl+P` | Fuzzy file finder |
| `Ctrl+F` | Repository text search |
| `w` | Worktree picker |
| `W` | Toggle whitespace ignoring |
| `+` / `-` | Increase/decrease context |
| `?` | Help |
| `q` | Quit |

Review state is stored outside the repository in the platform-local application data directory and keyed by repository, merge base, and head commit.

## Design

See [docs/architecture.md](docs/architecture.md) and [docs/keybindings.md](docs/keybindings.md).

## Development

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked --all-features
```

Licensed under Apache-2.0.
