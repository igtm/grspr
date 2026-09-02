use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub bare: bool,
    pub detached: bool,
    pub locked: bool,
    pub prunable: bool,
}

pub fn parse_porcelain(input: &str) -> Vec<Worktree> {
    input
        .split("\n\n")
        .filter_map(|block| {
            let mut path = None;
            let mut head = None;
            let mut branch = None;
            let mut bare = false;
            let mut detached = false;
            let mut locked = false;
            let mut prunable = false;
            for line in block.lines() {
                let (key, value) = line.split_once(' ').unwrap_or((line, ""));
                match key {
                    "worktree" => path = Some(PathBuf::from(value)),
                    "HEAD" => head = Some(value.to_owned()),
                    "branch" => {
                        branch = Some(
                            value
                                .strip_prefix("refs/heads/")
                                .unwrap_or(value)
                                .to_owned(),
                        )
                    }
                    "bare" => bare = true,
                    "detached" => detached = true,
                    "locked" => locked = true,
                    "prunable" => prunable = true,
                    _ => {}
                }
            }
            Some(Worktree {
                path: path?,
                head,
                branch,
                bare,
                detached,
                locked,
                prunable,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_worktrees() {
        let input = "worktree /repo\nHEAD abc\nbranch refs/heads/main\n\nworktree /tmp/feature\nHEAD def\ndetached\nlocked reason\n";
        let trees = parse_porcelain(input);
        assert_eq!(trees.len(), 2);
        assert_eq!(trees[0].branch.as_deref(), Some("main"));
        assert!(trees[1].detached);
        assert!(trees[1].locked);
    }
}
