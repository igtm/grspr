use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "grspr",
    version,
    about = "Grasp the whole PR from your terminal"
)]
pub struct Cli {
    /// PR-style revision range, for example main...HEAD
    pub range: Option<String>,

    /// Base revision (alternative to RANGE)
    #[arg(long)]
    pub base: Option<String>,

    /// Head revision (defaults to HEAD)
    #[arg(long)]
    pub head: Option<String>,

    /// Repository or worktree to review
    #[arg(short = 'C', long, default_value = ".")]
    pub repo: PathBuf,

    /// Unified diff context lines
    #[arg(short = 'U', long, default_value_t = 3)]
    pub context: u16,

    /// Ignore whitespace changes
    #[arg(short = 'w', long)]
    pub ignore_whitespace: bool,

    /// Disable mouse capture
    #[arg(long)]
    pub no_mouse: bool,
}

impl Cli {
    pub fn revisions(&self) -> anyhow::Result<(Option<String>, String)> {
        if self.range.is_some() && (self.base.is_some() || self.head.is_some()) {
            anyhow::bail!("RANGE cannot be combined with --base or --head");
        }
        if let Some(range) = &self.range {
            let Some((base, head)) = range.split_once("...") else {
                anyhow::bail!("RANGE must use PR semantics: BASE...HEAD");
            };
            if base.is_empty() || head.is_empty() {
                anyhow::bail!("both BASE and HEAD are required in RANGE");
            }
            return Ok((Some(base.to_owned()), head.to_owned()));
        }
        Ok((
            self.base.clone(),
            self.head.clone().unwrap_or_else(|| "HEAD".into()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn parses_triple_dot_range() {
        let cli = Cli::try_parse_from(["grspr", "main...feature"]).unwrap();
        assert_eq!(
            cli.revisions().unwrap(),
            (Some("main".into()), "feature".into())
        );
    }

    #[test]
    fn rejects_two_dot_range() {
        let cli = Cli::try_parse_from(["grspr", "main..feature"]).unwrap();
        assert!(cli.revisions().is_err());
    }
}
