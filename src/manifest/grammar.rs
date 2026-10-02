use anyhow::{Result, bail};
use std::fmt;

/// A package as named in a manifest: a name, optionally pinned to a version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackageSpec {
    pub name: String,
    pub version: Option<String>,
}

impl PackageSpec {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: None,
        }
    }

    pub fn pinned(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: Some(version.into()),
        }
    }

    /// Whitespace is the only separator, because it is the one character no
    /// package manager allows inside a package name. Names are therefore
    /// opaque: `node@20` is a Homebrew formula, not a pinned `node`.
    pub fn parse(text: &str) -> Result<Self> {
        let mut fields = text.split_whitespace();
        let Some(name) = fields.next() else {
            bail!("empty package entry");
        };
        if name.starts_with("//") {
            bail!("`{}`: comments start with `#`", text.trim());
        }
        // Such a name reaches the manager as an option. Building commands as argv
        // stops a name changing a command's shape, but not its being read as a flag.
        if name.starts_with('-') {
            bail!("`{}`: a package name cannot start with `-`", text.trim());
        }
        let version = fields.next();
        if let Some(extra) = fields.next() {
            bail!(
                "`{}`: expected `<name>` or `<name> <version>`, found `{extra}`",
                text.trim()
            );
        }
        Ok(match version {
            Some(version) => Self::pinned(name, version),
            None => Self::new(name),
        })
    }
}

impl fmt::Display for PackageSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.version {
            Some(version) => write!(f, "{} {}", self.name, version),
            None => f.write_str(&self.name),
        }
    }
}

/// The comment keeps its `#` and the declaration keeps the whitespace that
/// separated them, so a line can be rewritten without disturbing its layout.
pub fn split_comment(line: &str) -> (&str, Option<&str>) {
    match line.find('#') {
        Some(at) => (&line[..at], Some(&line[at..])),
        None => (line, None),
    }
}

/// `Ok(None)` for blank and comment-only lines.
pub fn parse_line(line: &str) -> Result<Option<PackageSpec>> {
    let (declaration, _) = split_comment(line);
    if declaration.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(PackageSpec::parse(declaration)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(text: &str) -> PackageSpec {
        PackageSpec::parse(text).expect("parses")
    }

    fn line(text: &str) -> Option<PackageSpec> {
        parse_line(text).expect("parses")
    }

    #[test]
    fn a_second_field_is_the_version() {
        assert_eq!(
            spec("ripgrep 14.1.0"),
            PackageSpec::pinned("ripgrep", "14.1.0")
        );
        assert_eq!(
            spec("ripgrep\t14.1.0"),
            PackageSpec::pinned("ripgrep", "14.1.0")
        );
    }

    #[test]
    fn an_at_sign_is_part_of_the_name() {
        // Splitting on `@` turned the formula `node@20` into a pinned `node`,
        // and the npm scope `@scope/pkg` into an empty name.
        assert_eq!(spec("node@20"), PackageSpec::new("node@20"));
        assert_eq!(spec("@scope/pkg"), PackageSpec::new("@scope/pkg"));
    }

    #[test]
    fn a_third_field_is_rejected() {
        assert!(PackageSpec::parse("ripgrep 14.1.0 extra").is_err());
    }

    #[test]
    fn display_round_trips() {
        for text in ["ripgrep", "ripgrep 14.1.0", "node@20", "@scope/pkg 1.0.0"] {
            assert_eq!(spec(text).to_string(), text);
        }
    }

    #[test]
    fn blank_and_comment_lines_declare_nothing() {
        for text in ["", "   ", "# editors", "   # indented"] {
            assert_eq!(line(text), None);
        }
    }

    #[test]
    fn a_trailing_comment_is_ignored_when_parsing() {
        assert_eq!(line("vim # my editor"), Some(PackageSpec::new("vim")));
        assert_eq!(
            line("ripgrep 14.1.0 # locked"),
            Some(PackageSpec::pinned("ripgrep", "14.1.0"))
        );
    }

    #[test]
    fn a_name_cannot_start_with_a_dash() {
        // `pacman -S --noconfirm` is a very different command from installing a
        // package called `--noconfirm`.
        assert!(PackageSpec::parse("--noconfirm").is_err());
        assert_eq!(spec("ttf-fira-code"), PackageSpec::new("ttf-fira-code"));
    }

    #[test]
    fn double_slash_is_not_a_comment() {
        assert!(parse_line("// editors").is_err());
    }

    #[test]
    fn split_comment_keeps_the_separating_whitespace() {
        assert_eq!(
            split_comment("vim   # my editor"),
            ("vim   ", Some("# my editor"))
        );
        assert_eq!(split_comment("vim"), ("vim", None));
    }
}
