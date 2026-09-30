use anyhow::Result;
use std::io::{IsTerminal, Write};
use std::sync::OnceLock;

use crate::reconcile::Reconciliation;

fn colored() -> bool {
    static COLORED: OnceLock<bool> = OnceLock::new();
    *COLORED.get_or_init(|| {
        if std::env::var_os("NO_COLOR").is_some() {
            return false;
        }
        std::io::stdout().is_terminal()
    })
}

fn paint(code: &str, text: &str) -> String {
    if colored() { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_string() }
}

pub fn bold(text: &str) -> String {
    paint("1", text)
}

pub fn dim(text: &str) -> String {
    paint("2", text)
}

pub fn problem(text: &str) -> String {
    paint("33", text)
}

fn green(text: &str) -> String {
    paint("32", text)
}

fn red(text: &str) -> String {
    paint("31", text)
}

fn yellow(text: &str) -> String {
    paint("33", text)
}

/// Empty when there is nothing to say, so callers stay silent on a clean machine.
pub fn render(changes: &Reconciliation) -> String {
    if !changes.has_work() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str(&bold(&changes.manager));
    out.push('\n');

    for spec in &changes.install {
        out.push_str(&format!("  {} {}\n", green("+"), spec));
    }
    for change in &changes.repin {
        out.push_str(&format!(
            "  {} {} {}\n",
            yellow("~"),
            change.spec,
            dim(&format!("(installed {})", change.installed))
        ));
    }
    for name in &changes.remove {
        out.push_str(&format!("  {} {}\n", red("-"), name));
    }

    out
}

pub fn summarize(sets: &[Reconciliation]) -> String {
    let install: usize = sets.iter().map(|changes| changes.install.len()).sum();
    let repin: usize = sets.iter().map(|changes| changes.repin.len()).sum();
    let remove: usize = sets.iter().map(|changes| changes.remove.len()).sum();

    let mut parts = Vec::new();
    if install > 0 {
        parts.push(green(&format!("{install} to install")));
    }
    if repin > 0 {
        parts.push(yellow(&format!("{repin} to change")));
    }
    if remove > 0 {
        parts.push(red(&format!("{remove} to remove")));
    }
    if parts.is_empty() {
        return green("everything matches your manifests").to_string();
    }
    parts.join(", ")
}

/// A non-interactive stdin answers *no*: a pipeline must pass `--yes` rather
/// than have consent inferred from the absence of a terminal.
pub fn confirm(question: &str) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        println!("{question} [y/N] n {}", dim("(stdin is not a terminal; pass --yes to proceed)"));
        return Ok(false);
    }

    print!("{question} [y/N] ");
    std::io::stdout().flush()?;

    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer)? == 0 {
        println!();
        return Ok(false);
    }
    let answer = answer.trim().to_ascii_lowercase();
    Ok(answer == "y" || answer == "yes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::Repin;
    use crate::manifest::grammar::PackageSpec;

    fn changes() -> Reconciliation {
        Reconciliation {
            manager: "pacman".into(),
            install: vec![PackageSpec::new("ripgrep")],
            repin: vec![Repin { spec: PackageSpec::pinned("bat", "0.24.0"), installed: "0.23.0".into() }],
            remove: vec!["nano".into()],
        }
    }

    #[test]
    fn a_clean_plan_renders_nothing() {
        let quiet = Reconciliation { manager: "pacman".into(), ..Reconciliation::default() };
        assert!(render(&quiet).is_empty());
    }

    #[test]
    fn every_bucket_appears_in_the_rendering() {
        let text = render(&changes());
        assert!(text.contains("ripgrep"));
        assert!(text.contains("bat 0.24.0"));
        assert!(text.contains("installed 0.23.0"));
        assert!(text.contains("nano"));

    }

    #[test]
    fn the_summary_counts_each_bucket() {
        let text = summarize(&[changes()]);
        assert!(text.contains("1 to install"));
        assert!(text.contains("1 to change"));
        assert!(text.contains("1 to remove"));

    }

    #[test]
    fn a_clean_summary_says_so() {
        let quiet = Reconciliation { manager: "pacman".into(), ..Reconciliation::default() };
        assert!(summarize(&[quiet]).contains("everything matches"));
    }
}
