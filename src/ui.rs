use std::sync::LazyLock;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Tabs, Wrap,
    },
};
use syntect::{
    easy::HighlightLines,
    highlighting::{Style as SyntectStyle, ThemeSet},
    parsing::SyntaxSet,
};

use crate::{
    app::{App, Focus, Overlay},
    diff::DiffLineKind,
    git::PullRequestStatus,
    review::ReviewStatus,
};

const ACCENT: Color = Color::Rgb(122, 162, 247);
const GREEN: Color = Color::Rgb(158, 206, 106);
const RED: Color = Color::Rgb(247, 118, 142);
const MUTED: Color = Color::Rgb(86, 95, 137);
const YELLOW: Color = Color::Rgb(224, 175, 104);
static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(area);
    render_header(frame, rows[0], app);
    render_body(frame, rows[1], app);
    render_footer(frame, rows[2], app);
    match app.overlay {
        Overlay::None => {}
        Overlay::Finder => render_finder(frame, app),
        Overlay::SearchInput => render_search_input(frame, app),
        Overlay::SearchResults => render_search_results(frame, app),
        Overlay::Worktrees => render_worktrees(frame, app),
        Overlay::Preview => render_preview(frame, app),
        Overlay::Help => render_help(frame),
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let titles = [" PR Diff ", " File ", " Search ", " Worktrees "];
    let selected = match app.overlay {
        Overlay::Finder | Overlay::Preview => 1,
        Overlay::SearchInput | Overlay::SearchResults => 2,
        Overlay::Worktrees => 3,
        _ => 0,
    };
    frame.render_widget(
        Tabs::new(titles)
            .select(selected)
            .highlight_style(Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
        area,
    );
}

fn render_body(frame: &mut Frame, area: Rect, app: &App) {
    if area.width < 72 {
        if app.focus == Focus::Files {
            render_files(frame, area, app);
        } else {
            render_diff(frame, area, app);
        }
        return;
    }
    let sidebar = (area.width / 3).clamp(24, 48);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(sidebar), Constraint::Min(30)])
        .split(area);
    render_files(frame, columns[0], app);
    render_diff(frame, columns[1], app);
}

fn render_files(frame: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .files
        .iter()
        .map(|file| {
            let (review, color) = match app.review_status(&file.path) {
                ReviewStatus::Unreviewed => ("○", MUTED),
                ReviewStatus::Viewed => ("◐", YELLOW),
                ReviewStatus::Reviewed => ("✓", GREEN),
            };
            let stats = match (file.additions, file.deletions) {
                (Some(add), Some(del)) => format!(" +{add} -{del}"),
                _ => String::new(),
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {review} "), Style::new().fg(color)),
                Span::styled(format!("{} ", file.kind.symbol()), Style::new().fg(MUTED)),
                Span::raw(file.path.to_string_lossy()),
                Span::styled(stats, Style::new().fg(MUTED)),
            ]))
        })
        .collect();
    let border = if app.focus == Focus::Files {
        ACCENT
    } else {
        MUTED
    };
    let list = List::new(items)
        .block(
            Block::new()
                .title(format!(" Changed Files ({}) ", app.files.len()))
                .borders(Borders::ALL)
                .border_style(Style::new().fg(border)),
        )
        .highlight_style(
            Style::new()
                .bg(Color::Rgb(41, 46, 66))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸");
    let mut state = ListState::default().with_selected(if app.files.is_empty() {
        None
    } else {
        Some(app.selected_file)
    });
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_diff(frame: &mut Frame, area: Rect, app: &App) {
    if app.files.is_empty() {
        render_empty_diff(frame, area, app);
        return;
    }
    let path = app
        .files
        .get(app.selected_file)
        .map(|file| file.path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "No changed files".into());
    let height = usize::from(area.height.saturating_sub(2));
    let syntax = SYNTAXES
        .find_syntax_for_file(&path)
        .ok()
        .flatten()
        .unwrap_or_else(|| SYNTAXES.find_syntax_plain_text());
    let theme = &THEMES.themes["base16-ocean.dark"];
    let mut highlighter = HighlightLines::new(syntax, theme);
    let lines: Vec<Line> = app
        .diff
        .lines
        .iter()
        .skip(app.diff_scroll)
        .take(height)
        .map(|line| {
            let numbers = match (line.old_line, line.new_line) {
                (Some(old), Some(new)) => format!("{old:>5} {new:>5} │"),
                (Some(old), None) => format!("{old:>5}       │"),
                (None, Some(new)) => format!("      {new:>5} │"),
                _ => "            │".into(),
            };
            let style = match line.kind {
                DiffLineKind::Added => Style::new().fg(GREEN).bg(Color::Rgb(25, 50, 40)),
                DiffLineKind::Deleted => Style::new().fg(RED).bg(Color::Rgb(55, 30, 40)),
                DiffLineKind::Hunk => Style::new()
                    .fg(ACCENT)
                    .bg(Color::Rgb(35, 40, 65))
                    .add_modifier(Modifier::BOLD),
                DiffLineKind::Header => Style::new().fg(YELLOW),
                DiffLineKind::Meta => Style::new().fg(MUTED),
                DiffLineKind::Context => Style::default(),
            };
            let mut spans = vec![Span::styled(numbers, Style::new().fg(MUTED))];
            if matches!(
                line.kind,
                DiffLineKind::Added | DiffLineKind::Deleted | DiffLineKind::Context
            ) {
                let (marker, code) = line
                    .text
                    .split_at(line.text.chars().next().map(char::len_utf8).unwrap_or(0));
                spans.push(Span::styled(marker.to_owned(), style));
                match highlighter.highlight_line(code, &SYNTAXES) {
                    Ok(ranges) => spans.extend(ranges.into_iter().map(|(syntax_style, text)| {
                        Span::styled(
                            text.to_owned(),
                            syntax_style_to_tui(syntax_style, line.kind),
                        )
                    })),
                    Err(_) => spans.push(Span::styled(code.to_owned(), style)),
                }
            } else {
                spans.push(Span::styled(line.text.clone(), style));
            }
            Line::from(spans)
        })
        .collect();
    let title = if app.loading {
        format!(" {path} · loading… ")
    } else {
        format!(" {path} ")
    };
    let border = if app.focus == Focus::Diff {
        ACCENT
    } else {
        MUTED
    };
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::new()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::new().fg(border)),
        ),
        area,
    );
    let mut scrollbar_state = ScrollbarState::new(app.diff.lines.len()).position(app.diff_scroll);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight),
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );
}

fn render_empty_diff(frame: &mut Frame, area: Rect, app: &App) {
    let range = format!("{}...{}", app.repo.base, app.repo.head);
    let review_head = if app.repo.head == "HEAD" {
        &app.repo.branch
    } else {
        &app.repo.head
    };
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "No changed files",
            Style::new().fg(YELLOW).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("The committed Git diff for {range} is empty."),
            Style::new().fg(MUTED),
        )),
        Line::from(""),
    ];
    match &app.pull_request_status {
        PullRequestStatus::Checking => lines.push(Line::from(Span::styled(
            format!("Checking GitHub PR for review head {review_head}…"),
            Style::new().fg(ACCENT),
        ))),
        PullRequestStatus::Found(info) => {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("PR #{} exists", info.number),
                    Style::new().fg(GREEN).add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(" · {}", info.title)),
            ]));
            lines.push(Line::from(format!(
                "GitHub range: {}...{} · {}",
                info.base_ref, info.head_ref, info.state
            )));
            lines.push(Line::from(Span::styled(&info.url, Style::new().fg(ACCENT))));
            lines.push(Line::from(
                "The PR exists, but the selected local range has no diff.",
            ));
        }
        PullRequestStatus::NotFound => {
            lines.push(Line::from(Span::styled(
                format!("No GitHub PR found for review head {review_head}."),
                Style::new().fg(YELLOW).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(
                "Use an explicit BASE...HEAD range if you meant to review another branch.",
            ));
        }
        PullRequestStatus::Unavailable(reason) => {
            lines.push(Line::from(Span::styled(
                "GitHub PR status is unavailable.",
                Style::new().fg(YELLOW).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(reason, Style::new().fg(MUTED))));
            lines.push(Line::from(
                "The local range is still valid; install/authenticate gh to distinguish PR state.",
            ));
        }
        PullRequestStatus::NotChecked => {}
    }
    let border = if app.focus == Focus::Diff {
        ACCENT
    } else {
        MUTED
    };
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(
                Block::new()
                    .title(" Empty review range ")
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(border)),
            ),
        area,
    );
}

fn syntax_style_to_tui(style: SyntectStyle, kind: DiffLineKind) -> Style {
    let foreground = style.foreground;
    let background = match kind {
        DiffLineKind::Added => Color::Rgb(25, 50, 40),
        DiffLineKind::Deleted => Color::Rgb(55, 30, 40),
        _ => Color::Reset,
    };
    Style::new()
        .fg(Color::Rgb(foreground.r, foreground.g, foreground.b))
        .bg(background)
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let progress = format!(" {}/{} reviewed ", app.reviewed_count(), app.files.len());
    let left = format!(
        " {} [{}] {}...{} ",
        app.repo.branch,
        app.repo.root.display(),
        app.repo.base,
        app.repo.head
    );
    let hints = match app.focus {
        Focus::Files => {
            "j/k file  Enter preview  r reviewed  Tab panel  Ctrl+P files  Ctrl+F search  ? help"
        }
        Focus::Diff => {
            "j/k scroll  [/ ] hunk  p preview  W whitespace  +/- context  Tab panel  ? help"
        }
    };
    let widths = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(left.len().min(u16::MAX as usize) as u16),
            Constraint::Min(10),
            Constraint::Length(progress.len() as u16),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(left).style(Style::new().bg(Color::Rgb(36, 40, 59)).fg(ACCENT)),
        widths[0],
    );
    frame.render_widget(
        Paragraph::new(format!(" {hints} · {}", app.status))
            .style(Style::new().bg(Color::Rgb(36, 40, 59)))
            .alignment(Alignment::Center),
        widths[1],
    );
    frame.render_widget(
        Paragraph::new(progress).style(Style::new().bg(Color::Rgb(36, 40, 59)).fg(GREEN)),
        widths[2],
    );
}

fn popup(frame: &mut Frame, percent_x: u16, percent_y: u16) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(frame.area());
    let horizontal = Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1]);
    frame.render_widget(Clear, horizontal[1]);
    horizontal[1]
}

fn render_finder(frame: &mut Frame, app: &App) {
    let area = popup(frame, 75, 70);
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(3)]).split(area);
    frame.render_widget(
        Paragraph::new(format!("> {}", app.finder_query))
            .block(Block::bordered().title(" Open File · Ctrl+P ")),
        rows[0],
    );
    let items: Vec<ListItem> = app
        .finder_matches
        .iter()
        .filter_map(|index| app.tracked_files.get(*index))
        .map(|path| ListItem::new(path.to_string_lossy()))
        .collect();
    let mut state = ListState::default().with_selected(if items.is_empty() {
        None
    } else {
        Some(app.finder_selected)
    });
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::bordered())
            .highlight_symbol("▸ ")
            .highlight_style(Style::new().fg(ACCENT)),
        rows[1],
        &mut state,
    );
}

fn render_search_input(frame: &mut Frame, app: &App) {
    let area = popup(frame, 70, 20);
    frame.render_widget(
        Paragraph::new(format!("rg > {}", app.search_query))
            .block(Block::bordered().title(" Search Repository · Enter to run ")),
        area,
    );
}

fn render_search_results(frame: &mut Frame, app: &App) {
    let area = popup(frame, 85, 80);
    let items: Vec<ListItem> = app
        .search_results
        .iter()
        .map(|result| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(
                        "{}:{}:{} ",
                        result.path.display(),
                        result.line,
                        result.column
                    ),
                    Style::new().fg(ACCENT),
                ),
                Span::raw(&result.text),
            ]))
        })
        .collect();
    let mut state = ListState::default().with_selected(if items.is_empty() {
        None
    } else {
        Some(app.search_selected)
    });
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::bordered().title(format!(
                " Search: {} ({}) ",
                app.search_query,
                app.search_results.len()
            )))
            .highlight_symbol("▸ ")
            .highlight_style(Style::new().bg(Color::Rgb(41, 46, 66))),
        area,
        &mut state,
    );
}

fn render_worktrees(frame: &mut Frame, app: &App) {
    let area = popup(frame, 80, 65);
    let items: Vec<ListItem> = app
        .worktrees
        .iter()
        .map(|tree| {
            let current = if tree.path == app.repo.root {
                "●"
            } else {
                " "
            };
            let branch =
                tree.branch
                    .as_deref()
                    .unwrap_or(if tree.detached { "detached" } else { "bare" });
            let flags = if tree.locked {
                " locked"
            } else if tree.prunable {
                " prunable"
            } else {
                ""
            };
            ListItem::new(format!(
                "{current} {branch:<24} {}{flags}",
                tree.path.display()
            ))
        })
        .collect();
    let mut state = ListState::default().with_selected(if items.is_empty() {
        None
    } else {
        Some(app.worktree_selected)
    });
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::bordered().title(" Worktrees · Enter switch context "))
            .highlight_symbol("▸ ")
            .highlight_style(Style::new().fg(ACCENT)),
        area,
        &mut state,
    );
}

fn render_preview(frame: &mut Frame, app: &App) {
    let area = popup(frame, 90, 88);
    let height = usize::from(area.height.saturating_sub(2));
    let lines: Vec<Line> = app
        .preview_lines
        .iter()
        .enumerate()
        .skip(app.preview_scroll)
        .take(height)
        .map(|(index, text)| {
            Line::from(vec![
                Span::styled(format!("{:>6} │ ", index + 1), Style::new().fg(MUTED)),
                Span::raw(text),
            ])
        })
        .collect();
    let title = format!(
        " Preview: {} · Esc back ",
        app.preview_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default()
    );
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().title(title))
            .wrap(Wrap { trim: false }),
        area,
    );
    let mut scrollbar_state =
        ScrollbarState::new(app.preview_lines.len()).position(app.preview_scroll);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight),
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );
}

fn render_help(frame: &mut Frame) {
    let area = popup(frame, 70, 75);
    let text = vec![
        Line::from("Navigation"),
        Line::from("  j/k or arrows   move/scroll"),
        Line::from("  J/K             previous/next changed file"),
        Line::from("  [ / ]           previous/next hunk"),
        Line::from("  Tab             switch panel"),
        Line::from(""),
        Line::from("Review"),
        Line::from("  p/Space/Enter   preview file"),
        Line::from("  r               toggle reviewed"),
        Line::from("  W               ignore whitespace"),
        Line::from("  +/-             context lines"),
        Line::from(""),
        Line::from("Explore"),
        Line::from("  Ctrl+P          fuzzy file finder"),
        Line::from("  Ctrl+F          ripgrep search"),
        Line::from("  w               worktrees"),
        Line::from("  ?               close help"),
        Line::from("  q               quit"),
    ];
    frame.render_widget(
        Paragraph::new(text).block(Block::bordered().title(" grspr Help ")),
        area,
    );
}
