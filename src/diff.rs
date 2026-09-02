use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
    Unknown,
}

impl ChangeKind {
    pub fn from_status(status: &str) -> Self {
        match status.as_bytes().first().copied() {
            Some(b'A') => Self::Added,
            Some(b'M') => Self::Modified,
            Some(b'D') => Self::Deleted,
            Some(b'R') => Self::Renamed,
            Some(b'C') => Self::Copied,
            Some(b'T') => Self::TypeChanged,
            Some(b'U') => Self::Unmerged,
            _ => Self::Unknown,
        }
    }

    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Copied => "C",
            Self::TypeChanged => "T",
            Self::Unmerged => "U",
            Self::Unknown => "?",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    pub old_path: Option<PathBuf>,
    pub path: PathBuf,
    pub kind: ChangeKind,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Header,
    Hunk,
    Added,
    Deleted,
    Context,
    Meta,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub text: String,
    pub kind: DiffLineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}

#[derive(Debug, Default, Clone)]
pub struct DiffDocument {
    pub lines: Vec<DiffLine>,
    pub hunks: Vec<usize>,
    old_line: Option<u32>,
    new_line: Option<u32>,
}

impl DiffDocument {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn push(&mut self, text: String) {
        let kind = classify_line(&text);
        if kind == DiffLineKind::Hunk {
            self.hunks.push(self.lines.len());
            if let Some((old, new)) = parse_hunk_header(&text) {
                self.old_line = Some(old);
                self.new_line = Some(new);
            }
        }

        let (old_line, new_line) = match kind {
            DiffLineKind::Added => {
                let value = self.new_line;
                self.new_line = self.new_line.map(|line| line + 1);
                (None, value)
            }
            DiffLineKind::Deleted => {
                let value = self.old_line;
                self.old_line = self.old_line.map(|line| line + 1);
                (value, None)
            }
            DiffLineKind::Context => {
                let old = self.old_line;
                let new = self.new_line;
                self.old_line = self.old_line.map(|line| line + 1);
                self.new_line = self.new_line.map(|line| line + 1);
                (old, new)
            }
            _ => (None, None),
        };
        self.lines.push(DiffLine {
            text,
            kind,
            old_line,
            new_line,
        });
    }
}

pub fn classify_line(line: &str) -> DiffLineKind {
    if line.starts_with("@@") {
        DiffLineKind::Hunk
    } else if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff --git") {
        DiffLineKind::Header
    } else if line.starts_with('+') {
        DiffLineKind::Added
    } else if line.starts_with('-') {
        DiffLineKind::Deleted
    } else if line.starts_with(' ') {
        DiffLineKind::Context
    } else {
        DiffLineKind::Meta
    }
}

fn parse_hunk_header(line: &str) -> Option<(u32, u32)> {
    let mut parts = line.split_whitespace();
    parts.next()?;
    let old = parts
        .next()?
        .strip_prefix('-')?
        .split(',')
        .next()?
        .parse()
        .ok()?;
    let new = parts
        .next()?
        .strip_prefix('+')?
        .split(',')
        .next()?
        .parse()
        .ok()?;
    Some((old, new))
}

pub fn parse_name_status_z(bytes: &[u8]) -> anyhow::Result<Vec<ChangedFile>> {
    let fields: Vec<&[u8]> = bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .collect();
    let mut files = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let status = String::from_utf8_lossy(fields[index]);
        index += 1;
        let kind = ChangeKind::from_status(&status);
        if matches!(kind, ChangeKind::Renamed | ChangeKind::Copied) {
            let old = fields
                .get(index)
                .ok_or_else(|| anyhow::anyhow!("missing old rename path"))?;
            let new = fields
                .get(index + 1)
                .ok_or_else(|| anyhow::anyhow!("missing new rename path"))?;
            index += 2;
            files.push(ChangedFile {
                old_path: Some(PathBuf::from(String::from_utf8_lossy(old).into_owned())),
                path: PathBuf::from(String::from_utf8_lossy(new).into_owned()),
                kind,
                additions: None,
                deletions: None,
            });
        } else {
            let path = fields
                .get(index)
                .ok_or_else(|| anyhow::anyhow!("missing changed path"))?;
            index += 1;
            files.push(ChangedFile {
                old_path: None,
                path: PathBuf::from(String::from_utf8_lossy(path).into_owned()),
                kind,
                additions: None,
                deletions: None,
            });
        }
    }
    Ok(files)
}

pub fn apply_numstat_z(files: &mut [ChangedFile], bytes: &[u8]) {
    let fields: Vec<&[u8]> = bytes.split(|byte| *byte == 0).collect();
    let mut index = 0;
    while index < fields.len() {
        let field = fields[index];
        index += 1;
        if field.is_empty() {
            continue;
        }
        let mut pieces = field.splitn(3, |byte| *byte == b'\t');
        let Some(additions) = pieces.next() else {
            continue;
        };
        let Some(deletions) = pieces.next() else {
            continue;
        };
        let Some(path) = pieces.next() else { continue };
        let target = if path.is_empty() {
            index += 1; // old path
            let Some(new_path) = fields.get(index) else {
                break;
            };
            index += 1;
            *new_path
        } else {
            path
        };
        let target = PathBuf::from(String::from_utf8_lossy(target).into_owned());
        if let Some(file) = files.iter_mut().find(|file| file.path == target) {
            file.additions = parse_stat(additions);
            file.deletions = parse_stat(deletions);
        }
    }
}

fn parse_stat(value: &[u8]) -> Option<u32> {
    if value == b"-" {
        None
    } else {
        std::str::from_utf8(value).ok()?.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_statuses_and_rename() {
        let input = b"A\0new.rs\0M\0src/lib.rs\0D\0old.rs\0R100\0before.rs\0after.rs\0";
        let files = parse_name_status_z(input).unwrap();
        assert_eq!(files.len(), 4);
        assert_eq!(files[0].kind, ChangeKind::Added);
        assert_eq!(files[2].kind, ChangeKind::Deleted);
        assert_eq!(
            files[3].old_path.as_deref(),
            Some(std::path::Path::new("before.rs"))
        );
        assert_eq!(files[3].path, PathBuf::from("after.rs"));
    }

    #[test]
    fn tracks_hunks_and_line_numbers() {
        let mut doc = DiffDocument::default();
        for line in ["@@ -10,2 +20,3 @@", " old", "-gone", "+new", "+more"] {
            doc.push(line.into());
        }
        assert_eq!(doc.hunks, vec![0]);
        assert_eq!(
            (doc.lines[1].old_line, doc.lines[1].new_line),
            (Some(10), Some(20))
        );
        assert_eq!(
            (doc.lines[2].old_line, doc.lines[2].new_line),
            (Some(11), None)
        );
        assert_eq!(
            (doc.lines[3].old_line, doc.lines[3].new_line),
            (None, Some(21))
        );
    }

    #[test]
    fn applies_binary_and_rename_numstat() {
        let mut files = parse_name_status_z(b"R100\0a.rs\0b.rs\0M\0image.png\0").unwrap();
        apply_numstat_z(&mut files, b"5\t2\t\0a.rs\0b.rs\0-\t-\timage.png\0");
        assert_eq!((files[0].additions, files[0].deletions), (Some(5), Some(2)));
        assert_eq!((files[1].additions, files[1].deletions), (None, None));
    }

    #[test]
    fn parses_a_large_diff_incrementally() {
        let mut document = DiffDocument::default();
        document.push("@@ -1,50000 +1,50000 @@".into());
        for index in 0..50_000 {
            document.push(format!("+line {index}"));
        }
        assert_eq!(document.lines.len(), 50_001);
        assert_eq!(document.lines.last().unwrap().new_line, Some(50_000));
    }
}
