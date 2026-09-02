use std::{fs, path::Path, process::Command};

use grspr::{
    diff::ChangeKind,
    git::{GitCli, WorkerEvent},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn reads_a_real_pr_style_diff() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.email", "test@example.com"]);
    git(root, &["config", "user.name", "grspr test"]);
    fs::write(root.join("modified.txt"), "before\n").unwrap();
    fs::write(root.join("deleted.txt"), "gone\n").unwrap();
    fs::write(root.join("rename-me.txt"), "rename\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "base"]);
    git(root, &["switch", "-c", "feature"]);
    fs::write(root.join("modified.txt"), "before\nafter\n").unwrap();
    fs::write(root.join("added.txt"), "new\n").unwrap();
    fs::remove_file(root.join("deleted.txt")).unwrap();
    git(root, &["mv", "rename-me.txt", "renamed.txt"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "feature"]);

    let repository = GitCli::discover(root, Some("main".into()), "HEAD".into())
        .await
        .unwrap();
    let files = GitCli::changed_files(&repository).await.unwrap();
    assert_eq!(files.len(), 4);
    assert!(
        files
            .iter()
            .any(|file| file.path == Path::new("added.txt") && file.kind == ChangeKind::Added)
    );
    assert!(
        files
            .iter()
            .any(|file| file.path == Path::new("deleted.txt") && file.kind == ChangeKind::Deleted)
    );
    assert!(
        files
            .iter()
            .any(|file| file.path == Path::new("renamed.txt") && file.kind == ChangeKind::Renamed)
    );
    let modified = files
        .iter()
        .find(|file| file.path == Path::new("modified.txt"))
        .unwrap();
    assert_eq!((modified.additions, modified.deletions), (Some(1), Some(0)));
}

#[tokio::test]
async fn cancels_a_streaming_diff_without_reporting_completion() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.email", "grspr@example.invalid"]);
    git(root, &["config", "user.name", "grspr test"]);
    std::fs::write(root.join("large.txt"), "before\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "base"]);
    git(root, &["switch", "-c", "feature"]);
    std::fs::write(root.join("large.txt"), "after\n".repeat(100_000)).unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "large feature"]);

    let repository = GitCli::discover(root, Some("main".into()), "HEAD".into())
        .await
        .unwrap();
    let (tx, mut rx) = mpsc::channel(8);
    let cancel = CancellationToken::new();
    cancel.cancel();
    GitCli::stream_diff(
        repository,
        "large.txt".into(),
        3,
        false,
        1,
        tx.clone(),
        cancel,
    );
    drop(tx);

    while let Some(event) = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("cancelled diff task did not stop")
    {
        assert!(!matches!(event, WorkerEvent::DiffFinished { .. }));
    }
}
