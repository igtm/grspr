use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, Result};
use serde::Deserialize;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;

use crate::{
    diff::{ChangedFile, apply_numstat_z, parse_name_status_z},
    worktree::{Worktree, parse_porcelain},
};

#[derive(Debug, Clone)]
pub struct Repository {
    pub root: PathBuf,
    pub common_dir: PathBuf,
    pub base: String,
    pub head: String,
    pub base_oid: String,
    pub head_oid: String,
    pub merge_base_oid: String,
    pub branch: String,
}

#[derive(Debug, Clone)]
pub struct GitCli;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestInfo {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub url: String,
    pub base_ref: String,
    pub head_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullRequestStatus {
    NotChecked,
    Checking,
    Found(PullRequestInfo),
    NotFound,
    Unavailable(String),
}

impl GitCli {
    pub async fn discover(
        path: &Path,
        requested_base: Option<String>,
        head: String,
    ) -> Result<Repository> {
        let root = PathBuf::from(
            run_text(path, ["rev-parse", "--show-toplevel"])
                .await?
                .trim(),
        );
        let common = run_text(&root, ["rev-parse", "--git-common-dir"]).await?;
        let common_dir = {
            let path = PathBuf::from(common.trim());
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        };
        let base = match requested_base {
            Some(base) => base,
            None => detect_base(&root).await?,
        };
        let base_oid = rev_parse(&root, &base).await?;
        let head_oid = rev_parse(&root, &head).await?;
        let merge_base_oid = run_text(&root, ["merge-base", base_oid.as_str(), head_oid.as_str()])
            .await?
            .trim()
            .to_owned();
        let branch = run_text(&root, ["branch", "--show-current"])
            .await?
            .trim()
            .to_owned();
        Ok(Repository {
            root,
            common_dir,
            base,
            head,
            base_oid,
            head_oid,
            merge_base_oid,
            branch,
        })
    }

    pub async fn changed_files(repo: &Repository) -> Result<Vec<ChangedFile>> {
        let range = format!("{}...{}", repo.base_oid, repo.head_oid);
        let status = run_bytes(
            &repo.root,
            ["diff", "--name-status", "-z", "--find-renames", &range],
        )
        .await?;
        let stats = run_bytes(
            &repo.root,
            ["diff", "--numstat", "-z", "--find-renames", &range],
        )
        .await?;
        let mut files = parse_name_status_z(&status)?;
        apply_numstat_z(&mut files, &stats);
        Ok(files)
    }

    pub async fn tracked_files(repo: &Repository) -> Result<Vec<PathBuf>> {
        let bytes = run_bytes(&repo.root, ["ls-files", "-co", "--exclude-standard", "-z"]).await?;
        Ok(bytes
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(|path| PathBuf::from(String::from_utf8_lossy(path).into_owned()))
            .collect())
    }

    pub async fn worktrees(repo: &Repository) -> Result<Vec<Worktree>> {
        let text = run_text(&repo.root, ["worktree", "list", "--porcelain"]).await?;
        Ok(parse_porcelain(&text))
    }

    pub fn detect_pull_request(repo: Repository, tx: mpsc::Sender<WorkerEvent>) {
        tokio::spawn(async move {
            let target = if repo.head == "HEAD" {
                repo.branch.clone()
            } else {
                repo.head.clone()
            };
            if target.is_empty() {
                let _ = tx
                    .send(WorkerEvent::PullRequestStatusReady(
                        PullRequestStatus::Unavailable("detached HEAD".into()),
                    ))
                    .await;
                return;
            }

            let mut command = Command::new("gh");
            command
                .current_dir(&repo.root)
                .args(["pr", "view"])
                .arg(&target)
                .args(["--json", "number,title,state,url,baseRefName,headRefName"])
                .kill_on_drop(true);
            let status = match tokio::time::timeout(Duration::from_secs(5), command.output()).await
            {
                Ok(Ok(output)) if output.status.success() => parse_pull_request(&output.stdout)
                    .map(PullRequestStatus::Found)
                    .unwrap_or_else(|error| PullRequestStatus::Unavailable(error.to_string())),
                Ok(Ok(output)) => classify_pull_request_failure(&output.stderr),
                Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                    PullRequestStatus::Unavailable("gh is not installed".into())
                }
                Ok(Err(error)) => {
                    PullRequestStatus::Unavailable(format!("failed to start gh: {error}"))
                }
                Err(_) => PullRequestStatus::Unavailable("gh PR lookup timed out".into()),
            };
            let _ = tx.send(WorkerEvent::PullRequestStatusReady(status)).await;
        });
    }

    pub fn stream_diff(
        repo: Repository,
        path: PathBuf,
        context: u16,
        ignore_whitespace: bool,
        generation: u64,
        tx: mpsc::Sender<WorkerEvent>,
        cancel: CancellationToken,
    ) {
        tokio::spawn(async move {
            let range = format!("{}...{}", repo.base_oid, repo.head_oid);
            let mut command = Command::new("git");
            command
                .current_dir(&repo.root)
                .args(["--no-pager", "diff", "--no-ext-diff", "--no-color"])
                .arg(format!("--unified={context}"));
            if ignore_whitespace {
                command.arg("--ignore-all-space");
            }
            command
                .arg(range)
                .arg("--")
                .arg(&path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(error) => {
                    let _ = tx
                        .send(WorkerEvent::Failed {
                            generation,
                            message: error.to_string(),
                        })
                        .await;
                    return;
                }
            };
            let stdout = child.stdout.take().expect("stdout configured");
            let mut lines = BufReader::new(stdout).lines();
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => { let _ = child.kill().await; return; }
                    next = lines.next_line() => match next {
                        Ok(Some(line)) => { if tx.send(WorkerEvent::DiffLine { generation, line }).await.is_err() { return; } }
                        Ok(None) => break,
                        Err(error) => { let _ = tx.send(WorkerEvent::Failed { generation, message: error.to_string() }).await; return; }
                    }
                }
            }
            match child.wait().await {
                Ok(status) if status.success() => {
                    let _ = tx.send(WorkerEvent::DiffFinished { generation }).await;
                }
                Ok(status) => {
                    let _ = tx
                        .send(WorkerEvent::Failed {
                            generation,
                            message: format!("git diff exited with {status}"),
                        })
                        .await;
                }
                Err(error) => {
                    let _ = tx
                        .send(WorkerEvent::Failed {
                            generation,
                            message: error.to_string(),
                        })
                        .await;
                }
            }
        });
    }

    pub fn preview_file(
        repo: Repository,
        revision: String,
        path: PathBuf,
        generation: u64,
        tx: mpsc::Sender<WorkerEvent>,
    ) {
        tokio::spawn(async move {
            let spec = format!("{}:{}", revision, path.to_string_lossy());
            match run_text(&repo.root, ["show", spec.as_str()]).await {
                Ok(text) => {
                    let _ = tx
                        .send(WorkerEvent::PreviewReady {
                            generation,
                            path,
                            lines: text.lines().map(str::to_owned).collect(),
                        })
                        .await;
                }
                Err(error) => {
                    let _ = tx
                        .send(WorkerEvent::Failed {
                            generation,
                            message: error.to_string(),
                        })
                        .await;
                }
            }
        });
    }

    pub fn search(
        repo: Repository,
        query: String,
        generation: u64,
        tx: mpsc::Sender<WorkerEvent>,
        cancel: CancellationToken,
    ) {
        tokio::spawn(async move {
            let mut child = match Command::new("rg")
                .current_dir(&repo.root)
                .args([
                    "--line-number",
                    "--column",
                    "--no-heading",
                    "--color",
                    "never",
                    "--smart-case",
                    "--",
                ])
                .arg(&query)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
            {
                Ok(child) => child,
                Err(error) => {
                    let _ = tx
                        .send(WorkerEvent::Failed {
                            generation,
                            message: format!("failed to start rg: {error}"),
                        })
                        .await;
                    return;
                }
            };
            let stdout = child.stdout.take().expect("stdout configured");
            let mut lines = BufReader::new(stdout).lines();
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => { let _ = child.kill().await; return; }
                    next = lines.next_line() => match next {
                        Ok(Some(line)) => {
                            if let Some(result) = SearchResult::parse(&line)
                                && tx.send(WorkerEvent::SearchMatch { generation, result }).await.is_err()
                            {
                                return;
                            }
                        }
                        Ok(None) => break,
                        Err(error) => { let _ = tx.send(WorkerEvent::Failed { generation, message: error.to_string() }).await; return; }
                    }
                }
            }
            let _ = child.wait().await;
            let _ = tx.send(WorkerEvent::SearchFinished { generation }).await;
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub path: PathBuf,
    pub line: u32,
    pub column: u32,
    pub text: String,
}

impl SearchResult {
    pub fn parse(value: &str) -> Option<Self> {
        let mut parts = value.splitn(4, ':');
        Some(Self {
            path: PathBuf::from(parts.next()?),
            line: parts.next()?.parse().ok()?,
            column: parts.next()?.parse().ok()?,
            text: parts.next()?.to_owned(),
        })
    }
}

#[derive(Debug)]
pub enum WorkerEvent {
    DiffLine {
        generation: u64,
        line: String,
    },
    DiffFinished {
        generation: u64,
    },
    PreviewReady {
        generation: u64,
        path: PathBuf,
        lines: Vec<String>,
    },
    SearchMatch {
        generation: u64,
        result: SearchResult,
    },
    SearchFinished {
        generation: u64,
    },
    PullRequestStatusReady(PullRequestStatus),
    Failed {
        generation: u64,
        message: String,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPullRequest {
    number: u64,
    title: String,
    state: String,
    url: String,
    base_ref_name: String,
    head_ref_name: String,
}

fn parse_pull_request(bytes: &[u8]) -> Result<PullRequestInfo> {
    let value: GhPullRequest = serde_json::from_slice(bytes).context("invalid gh PR response")?;
    Ok(PullRequestInfo {
        number: value.number,
        title: value.title,
        state: value.state,
        url: value.url,
        base_ref: value.base_ref_name,
        head_ref: value.head_ref_name,
    })
}

fn classify_pull_request_failure(stderr: &[u8]) -> PullRequestStatus {
    let output = String::from_utf8_lossy(stderr);
    let message = output
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .trim()
        .to_owned();
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("no pull requests found")
        || normalized.contains("could not resolve to a pull request")
    {
        PullRequestStatus::NotFound
    } else {
        PullRequestStatus::Unavailable(if message.is_empty() {
            "gh could not determine PR status".into()
        } else {
            message
        })
    }
}

async fn detect_base(root: &Path) -> Result<String> {
    if let Ok(value) = run_text(
        root,
        [
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .await
    {
        let value = value.trim();
        if !value.is_empty() {
            return Ok(value.to_owned());
        }
    }
    for candidate in ["origin/main", "main", "origin/master", "master"] {
        if rev_parse(root, candidate).await.is_ok() {
            return Ok(candidate.to_owned());
        }
    }
    anyhow::bail!("could not detect a base branch; pass BASE...HEAD or --base")
}

async fn rev_parse(root: &Path, revision: &str) -> Result<String> {
    Ok(run_text(
        root,
        ["rev-parse", "--verify", &format!("{revision}^{{commit}}")],
    )
    .await?
    .trim()
    .to_owned())
}

async fn run_text<const N: usize>(root: &Path, args: [&str; N]) -> Result<String> {
    let bytes = run_bytes(root, args).await?;
    String::from_utf8(bytes).context("git returned non-UTF-8 text")
}

async fn run_bytes<const N: usize>(root: &Path, args: [&str; N]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(root)
        .arg("--no-pager")
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .await
        .context("failed to start git")?;
    if !output.status.success() {
        anyhow::bail!(
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::{
        PullRequestStatus, SearchResult, classify_pull_request_failure, parse_pull_request,
    };
    #[test]
    fn parses_search_result() {
        let result = SearchResult::parse("src/lib.rs:42:7:needle here").unwrap();
        assert_eq!(result.line, 42);
        assert_eq!(result.text, "needle here");
    }

    #[test]
    fn parses_pull_request_result() {
        let info = parse_pull_request(
            br#"{"number":42,"title":"Clear empty state","state":"OPEN","url":"https://example.invalid/42","baseRefName":"main","headRefName":"empty-state"}"#,
        )
        .unwrap();
        assert_eq!(info.number, 42);
        assert_eq!(info.base_ref, "main");
        assert_eq!(info.head_ref, "empty-state");
    }

    #[test]
    fn distinguishes_missing_pr_from_lookup_failure() {
        assert_eq!(
            classify_pull_request_failure(b"no pull requests found for branch"),
            PullRequestStatus::NotFound
        );
        assert!(matches!(
            classify_pull_request_failure(b"HTTP 503"),
            PullRequestStatus::Unavailable(_)
        ));
    }
}
