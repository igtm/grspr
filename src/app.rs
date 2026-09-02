use std::{io::Stdout, path::PathBuf, time::Duration};

use anyhow::Result;
use crossterm::event::{
    Event, EventStream, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
};
use futures::StreamExt;
use nucleo_matcher::{
    Config, Matcher,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Size};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    cli::Cli,
    diff::{ChangeKind, ChangedFile, DiffDocument},
    git::{GitCli, PullRequestStatus, Repository, SearchResult, WorkerEvent},
    review::{ReviewState, ReviewStatus, ReviewStore},
    ui,
    worktree::Worktree,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Files,
    Diff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    None,
    Finder,
    SearchInput,
    SearchResults,
    Worktrees,
    Preview,
    Help,
}

pub struct App {
    pub repo: Repository,
    pub files: Vec<ChangedFile>,
    pub selected_file: usize,
    pub focus: Focus,
    pub overlay: Overlay,
    pub diff: DiffDocument,
    pub diff_scroll: usize,
    pub loading: bool,
    pub context: u16,
    pub ignore_whitespace: bool,
    pub status: String,
    pub review: ReviewState,
    pub tracked_files: Vec<PathBuf>,
    pub finder_query: String,
    pub finder_matches: Vec<usize>,
    pub finder_selected: usize,
    pub search_query: String,
    pub search_results: Vec<SearchResult>,
    pub search_selected: usize,
    pub worktrees: Vec<Worktree>,
    pub worktree_selected: usize,
    pub preview_path: Option<PathBuf>,
    pub preview_lines: Vec<String>,
    pub preview_scroll: usize,
    pub terminal_size: Size,
    pub pull_request_status: PullRequestStatus,
    tx: mpsc::Sender<WorkerEvent>,
    rx: mpsc::Receiver<WorkerEvent>,
    review_store: ReviewStore,
    diff_generation: u64,
    preview_generation: u64,
    search_generation: u64,
    diff_cancel: CancellationToken,
    search_cancel: CancellationToken,
}

impl App {
    pub async fn load(cli: &Cli) -> Result<Self> {
        let (base, head) = cli.revisions()?;
        let repo = GitCli::discover(&cli.repo, base, head).await?;
        let files = GitCli::changed_files(&repo).await?;
        let tracked_files = GitCli::tracked_files(&repo).await.unwrap_or_default();
        let worktrees = GitCli::worktrees(&repo).await.unwrap_or_default();
        let review_store = ReviewStore::for_repository(&repo)?;
        let review = review_store.load().unwrap_or_default();
        let (tx, rx) = mpsc::channel(512);
        let mut app = Self {
            repo,
            files,
            selected_file: 0,
            focus: Focus::Files,
            overlay: Overlay::None,
            diff: DiffDocument::default(),
            diff_scroll: 0,
            loading: false,
            context: cli.context,
            ignore_whitespace: cli.ignore_whitespace,
            status: "Ready".into(),
            review,
            tracked_files,
            finder_query: String::new(),
            finder_matches: Vec::new(),
            finder_selected: 0,
            search_query: String::new(),
            search_results: Vec::new(),
            search_selected: 0,
            worktrees,
            worktree_selected: 0,
            preview_path: None,
            preview_lines: Vec::new(),
            preview_scroll: 0,
            terminal_size: Size::default(),
            pull_request_status: PullRequestStatus::NotChecked,
            tx,
            rx,
            review_store,
            diff_generation: 0,
            preview_generation: 0,
            search_generation: 0,
            diff_cancel: CancellationToken::new(),
            search_cancel: CancellationToken::new(),
        };
        if app.files.is_empty() {
            app.pull_request_status = PullRequestStatus::Checking;
            app.status = format!(
                "No diff in {}...{} · checking PR…",
                app.repo.base, app.repo.head
            );
            GitCli::detect_pull_request(app.repo.clone(), app.tx.clone());
        }
        app.mark_viewed();
        app.request_diff();
        Ok(app)
    }

    pub async fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        let mut events = EventStream::new();
        loop {
            self.terminal_size = terminal.size()?;
            terminal.draw(|frame| ui::render(frame, self))?;
            tokio::select! {
                maybe_event = events.next() => {
                    match maybe_event {
                        Some(Ok(Event::Key(key))) if key.kind == crossterm::event::KeyEventKind::Press => {
                            if self.handle_key(key) { break; }
                        }
                        Some(Ok(Event::Mouse(mouse))) => self.handle_mouse(mouse),
                        Some(Ok(_)) => {}
                        Some(Err(error)) => self.status = format!("input error: {error}"),
                        None => break,
                    }
                }
                Some(event) = self.rx.recv() => self.handle_worker(event),
                _ = tokio::time::sleep(Duration::from_millis(250)), if self.loading => {}
            }
        }
        Ok(())
    }

    pub fn review_status(&self, path: &PathBuf) -> ReviewStatus {
        self.review.files.get(path).copied().unwrap_or_default()
    }

    pub fn reviewed_count(&self) -> usize {
        self.files
            .iter()
            .filter(|file| self.review_status(&file.path) == ReviewStatus::Reviewed)
            .count()
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') => return true,
                KeyCode::Char('p') => {
                    self.open_finder();
                    return false;
                }
                KeyCode::Char('f') => {
                    self.overlay = Overlay::SearchInput;
                    self.search_query.clear();
                    return false;
                }
                _ => {}
            }
        }
        match self.overlay {
            Overlay::None => self.handle_main_key(key),
            Overlay::Finder => self.handle_finder_key(key),
            Overlay::SearchInput => self.handle_search_input(key),
            Overlay::SearchResults => self.handle_search_results_key(key),
            Overlay::Worktrees => self.handle_worktree_key(key),
            Overlay::Preview => self.handle_preview_key(key),
            Overlay::Help => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
                ) {
                    self.overlay = Overlay::None;
                }
                false
            }
        }
    }

    fn handle_main_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Tab => {
                self.focus = if self.focus == Focus::Files {
                    Focus::Diff
                } else {
                    Focus::Files
                }
            }
            KeyCode::BackTab => {
                self.focus = if self.focus == Focus::Files {
                    Focus::Diff
                } else {
                    Focus::Files
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.focus == Focus::Files {
                    self.select_relative(1)
                } else {
                    self.scroll_diff(1)
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.focus == Focus::Files {
                    self.select_relative(-1)
                } else {
                    self.scroll_diff(-1)
                }
            }
            KeyCode::Char('J') => self.select_relative(1),
            KeyCode::Char('K') => self.select_relative(-1),
            KeyCode::Char(']') => self.next_hunk(1),
            KeyCode::Char('[') => self.next_hunk(-1),
            KeyCode::PageDown => self.scroll_diff(20),
            KeyCode::PageUp => self.scroll_diff(-20),
            KeyCode::Home | KeyCode::Char('g') => self.diff_scroll = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.diff_scroll = self.diff.lines.len().saturating_sub(1)
            }
            KeyCode::Char('r') => self.toggle_reviewed(),
            KeyCode::Char('p') | KeyCode::Char(' ') | KeyCode::Enter => self.preview_selected(),
            KeyCode::Char('w') => {
                self.overlay = Overlay::Worktrees;
                self.worktree_selected = self.current_worktree_index();
            }
            KeyCode::Char('?') => self.overlay = Overlay::Help,
            KeyCode::Char('W') => {
                self.ignore_whitespace = !self.ignore_whitespace;
                self.request_diff();
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.context = self.context.saturating_add(1);
                self.request_diff();
            }
            KeyCode::Char('-') => {
                self.context = self.context.saturating_sub(1);
                self.request_diff();
            }
            _ => {}
        }
        false
    }

    fn handle_finder_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.finder_query.push(character);
                self.update_finder();
            }
            KeyCode::Backspace => {
                self.finder_query.pop();
                self.update_finder();
            }
            KeyCode::Down | KeyCode::Tab => {
                self.finder_selected =
                    (self.finder_selected + 1).min(self.finder_matches.len().saturating_sub(1))
            }
            KeyCode::Up | KeyCode::BackTab => {
                self.finder_selected = self.finder_selected.saturating_sub(1)
            }
            KeyCode::Enter => {
                if let Some(index) = self.finder_matches.get(self.finder_selected).copied() {
                    let path = self.tracked_files[index].clone();
                    self.request_preview(path);
                }
            }
            _ => {}
        }
        false
    }

    fn handle_search_input(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search_query.push(character)
            }
            KeyCode::Backspace => {
                self.search_query.pop();
            }
            KeyCode::Enter if !self.search_query.is_empty() => self.start_search(),
            _ => {}
        }
        false
    }

    fn handle_search_results_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Char('j') | KeyCode::Down => {
                self.search_selected =
                    (self.search_selected + 1).min(self.search_results.len().saturating_sub(1))
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.search_selected = self.search_selected.saturating_sub(1)
            }
            KeyCode::Enter | KeyCode::Char('p') => {
                if let Some(result) = self.search_results.get(self.search_selected) {
                    self.request_preview(result.path.clone());
                }
            }
            KeyCode::Char('/') => {
                self.overlay = Overlay::SearchInput;
                self.search_query.clear();
            }
            _ => {}
        }
        false
    }

    fn handle_worktree_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Char('j') | KeyCode::Down => {
                self.worktree_selected =
                    (self.worktree_selected + 1).min(self.worktrees.len().saturating_sub(1))
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.worktree_selected = self.worktree_selected.saturating_sub(1)
            }
            KeyCode::Enter => {
                if let Some(tree) = self.worktrees.get(self.worktree_selected) {
                    self.repo.root = tree.path.clone();
                    self.repo.branch = tree.branch.clone().unwrap_or_else(|| "detached".into());
                    self.status = format!("Switched context to {}", tree.path.display());
                    self.overlay = Overlay::None;
                    self.request_diff();
                }
            }
            _ => {}
        }
        false
    }

    fn handle_preview_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.overlay = Overlay::None,
            KeyCode::Char('j') | KeyCode::Down => {
                self.preview_scroll = self.preview_scroll.saturating_add(1)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.preview_scroll = self.preview_scroll.saturating_sub(1)
            }
            KeyCode::PageDown => self.preview_scroll = self.preview_scroll.saturating_add(20),
            KeyCode::PageUp => self.preview_scroll = self.preview_scroll.saturating_sub(20),
            KeyCode::Home | KeyCode::Char('g') => self.preview_scroll = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.preview_scroll = self.preview_lines.len().saturating_sub(1)
            }
            _ => {}
        }
        false
    }

    fn handle_mouse(&mut self, event: MouseEvent) {
        match event.kind {
            MouseEventKind::ScrollDown => {
                if self.overlay == Overlay::Preview {
                    self.preview_scroll = self.preview_scroll.saturating_add(3);
                } else {
                    self.scroll_diff(3);
                }
            }
            MouseEventKind::ScrollUp => {
                if self.overlay == Overlay::Preview {
                    self.preview_scroll = self.preview_scroll.saturating_sub(3);
                } else {
                    self.scroll_diff(-3);
                }
            }
            MouseEventKind::Down(_) if self.overlay == Overlay::None => {
                let sidebar = (self.terminal_size.width / 3).clamp(24, 48);
                if event.column < sidebar && event.row > 1 {
                    let index = usize::from(event.row.saturating_sub(2));
                    if index < self.files.len() {
                        self.selected_file = index;
                        self.mark_viewed();
                        self.request_diff();
                    }
                } else {
                    self.focus = Focus::Diff;
                }
            }
            _ => {}
        }
    }

    fn handle_worker(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::DiffLine { generation, line } if generation == self.diff_generation => {
                self.diff.push(line)
            }
            WorkerEvent::DiffFinished { generation } if generation == self.diff_generation => {
                self.loading = false;
                self.status = format!("{} hunks · context {}", self.diff.hunks.len(), self.context);
            }
            WorkerEvent::PreviewReady {
                generation,
                path,
                lines,
            } if generation == self.preview_generation => {
                self.preview_path = Some(path);
                self.preview_lines = lines;
                self.preview_scroll = 0;
                self.overlay = Overlay::Preview;
            }
            WorkerEvent::SearchMatch { generation, result }
                if generation == self.search_generation =>
            {
                self.search_results.push(result)
            }
            WorkerEvent::SearchFinished { generation } if generation == self.search_generation => {
                self.status = format!("{} matches", self.search_results.len())
            }
            WorkerEvent::PullRequestStatusReady(status) => {
                self.status = match &status {
                    PullRequestStatus::Found(info) => {
                        format!("PR #{} exists · selected range has no diff", info.number)
                    }
                    PullRequestStatus::NotFound => {
                        let head = if self.repo.head == "HEAD" {
                            &self.repo.branch
                        } else {
                            &self.repo.head
                        };
                        format!("No PR for {head} · selected range has no diff")
                    }
                    PullRequestStatus::Unavailable(_) => {
                        format!("No diff in {}...{}", self.repo.base, self.repo.head)
                    }
                    PullRequestStatus::NotChecked | PullRequestStatus::Checking => {
                        self.status.clone()
                    }
                };
                self.pull_request_status = status;
            }
            WorkerEvent::Failed { message, .. } => {
                self.loading = false;
                self.status = message;
            }
            _ => {}
        }
    }

    fn request_diff(&mut self) {
        let Some(file) = self.files.get(self.selected_file) else {
            return;
        };
        self.diff_cancel.cancel();
        self.diff_cancel = CancellationToken::new();
        self.diff_generation += 1;
        self.diff.clear();
        self.diff_scroll = 0;
        self.loading = true;
        self.status = format!("Loading {}…", file.path.display());
        GitCli::stream_diff(
            self.repo.clone(),
            file.path.clone(),
            self.context,
            self.ignore_whitespace,
            self.diff_generation,
            self.tx.clone(),
            self.diff_cancel.clone(),
        );
    }

    fn request_preview_at(&mut self, revision: String, path: PathBuf) {
        self.preview_generation += 1;
        self.status = format!("Previewing {}…", path.display());
        GitCli::preview_file(
            self.repo.clone(),
            revision,
            path,
            self.preview_generation,
            self.tx.clone(),
        );
    }

    fn request_preview(&mut self, path: PathBuf) {
        self.request_preview_at(self.repo.head_oid.clone(), path);
    }

    fn preview_selected(&mut self) {
        if let Some(file) = self.files.get(self.selected_file) {
            let (revision, path) = if file.kind == ChangeKind::Deleted {
                (
                    self.repo.base_oid.clone(),
                    file.old_path.clone().unwrap_or_else(|| file.path.clone()),
                )
            } else {
                (self.repo.head_oid.clone(), file.path.clone())
            };
            self.request_preview_at(revision, path);
        }
    }

    fn select_relative(&mut self, delta: isize) {
        if self.files.is_empty() {
            return;
        }
        self.selected_file = self
            .selected_file
            .saturating_add_signed(delta)
            .min(self.files.len() - 1);
        self.mark_viewed();
        self.request_diff();
    }

    fn scroll_diff(&mut self, delta: isize) {
        self.diff_scroll = self
            .diff_scroll
            .saturating_add_signed(delta)
            .min(self.diff.lines.len().saturating_sub(1));
    }

    fn next_hunk(&mut self, delta: isize) {
        if self.diff.hunks.is_empty() {
            return;
        }
        if delta > 0 {
            if let Some(position) = self
                .diff
                .hunks
                .iter()
                .copied()
                .find(|position| *position > self.diff_scroll)
            {
                self.diff_scroll = position;
            }
        } else if let Some(position) = self
            .diff
            .hunks
            .iter()
            .rev()
            .copied()
            .find(|position| *position < self.diff_scroll)
        {
            self.diff_scroll = position;
        }
    }

    fn mark_viewed(&mut self) {
        if let Some(file) = self.files.get(self.selected_file) {
            self.review
                .files
                .entry(file.path.clone())
                .or_insert(ReviewStatus::Viewed);
            let _ = self.review_store.save(&self.review);
        }
    }

    fn toggle_reviewed(&mut self) {
        let Some(file) = self.files.get(self.selected_file) else {
            return;
        };
        let new_status = {
            let status = self.review.files.entry(file.path.clone()).or_default();
            *status = if *status == ReviewStatus::Reviewed {
                ReviewStatus::Viewed
            } else {
                ReviewStatus::Reviewed
            };
            *status
        };
        match self.review_store.save(&self.review) {
            Ok(()) => self.status = format!("{}: {:?}", file.path.display(), new_status),
            Err(error) => self.status = error.to_string(),
        }
    }

    fn open_finder(&mut self) {
        self.overlay = Overlay::Finder;
        self.finder_query.clear();
        self.finder_matches = (0..self.tracked_files.len()).take(100).collect();
        self.finder_selected = 0;
    }

    fn update_finder(&mut self) {
        self.finder_selected = 0;
        if self.finder_query.is_empty() {
            self.finder_matches = (0..self.tracked_files.len()).take(100).collect();
            return;
        }
        let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
        let pattern = Pattern::new(
            &self.finder_query,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let values: Vec<&str> = self
            .tracked_files
            .iter()
            .map(|path| path.to_str().unwrap_or_default())
            .collect();
        self.finder_matches = pattern
            .match_list(values, &mut matcher)
            .into_iter()
            .take(100)
            .filter_map(|(matched, _)| {
                self.tracked_files
                    .iter()
                    .position(|path| path.to_str() == Some(matched))
            })
            .collect();
    }

    fn start_search(&mut self) {
        self.search_cancel.cancel();
        self.search_cancel = CancellationToken::new();
        self.search_generation += 1;
        self.search_results.clear();
        self.search_selected = 0;
        self.overlay = Overlay::SearchResults;
        self.status = format!("Searching for {}…", self.search_query);
        GitCli::search(
            self.repo.clone(),
            self.search_query.clone(),
            self.search_generation,
            self.tx.clone(),
            self.search_cancel.clone(),
        );
    }

    fn current_worktree_index(&self) -> usize {
        self.worktrees
            .iter()
            .position(|tree| tree.path == self.repo.root)
            .unwrap_or(0)
    }
}
