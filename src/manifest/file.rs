use anyhow::{Context, Result, bail};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::manifest::grammar::{self, PackageSpec};

/// One physical line, kept verbatim so comments, spacing and ordering survive
/// an edit. Anything mpm does not understand is passed through untouched.
#[derive(Debug, Clone)]
enum Line {
    Other(String),
    Declared { spec: PackageSpec, raw: String },
}

impl Line {
    fn raw(&self) -> &str {
        match self {
            Line::Other(raw) => raw,
            Line::Declared { raw, .. } => raw,
        }
    }
}

#[derive(Debug)]
pub struct ManifestFile {
    path: PathBuf,
    lines: Vec<Line>,
}

impl ManifestFile {
    /// A missing file is an empty manifest; a malformed entry is an error
    /// reported with its line number.
    pub fn load(path: &Path) -> Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(error).with_context(|| format!("could not read {}", path.display()));
            }
        };

        let mut lines = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let parsed = grammar::parse_line(line)
                .with_context(|| format!("{}:{}", path.display(), index + 1))?;
            lines.push(match parsed {
                Some(spec) => Line::Declared { spec, raw: line.to_string() },
                None => Line::Other(line.to_string()),
            });
        }

        Ok(Self { path: path.to_path_buf(), lines })
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    pub fn specs(&self) -> impl Iterator<Item = &PackageSpec> {
        self.lines.iter().filter_map(|line| match line {
            Line::Declared { spec, .. } => Some(spec),
            Line::Other(_) => None,
        })
    }

    pub fn declares(&self, name: &str) -> bool {
        self.specs().any(|spec| spec.name == name)
    }

    /// Returns whether anything changed. New packages are appended rather than
    /// sorted in, so the user's grouping survives, and rewriting an existing
    /// line keeps its trailing comment.
    pub fn declare(&mut self, wanted: &PackageSpec) -> bool {
        for line in &mut self.lines {
            let Line::Declared { spec, raw } = line else { continue };
            if spec.name != wanted.name {
                continue;
            }
            if spec == wanted {
                return false;
            }
            *spec = wanted.clone();
            *raw = rewrite(raw, wanted);
            return true;
        }
        self.lines.push(Line::Declared { spec: wanted.clone(), raw: wanted.to_string() });
        true
    }

    /// Returns whether anything changed.
    pub fn undeclare(&mut self, name: &str) -> bool {
        let before = self.lines.len();
        self.lines.retain(|line| !matches!(line, Line::Declared { spec, .. } if spec.name == name));
        self.lines.len() != before
    }

    /// Written to a sibling temporary file, flushed, then renamed over the
    /// target, so an interrupted write cannot truncate a package list.
    pub fn save(&self) -> Result<()> {
        let parent = self
            .path
            .parent()
            .with_context(|| format!("{} has no parent directory", self.path.display()))?;
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;

        let Some(file_name) = self.path.file_name().and_then(|name| name.to_str()) else {
            bail!("{} is not a usable manifest path", self.path.display());
        };
        let temp = parent.join(format!(".{file_name}.mpm-tmp"));

        {
            let mut handle = fs::File::create(&temp)
                .with_context(|| format!("could not create {}", temp.display()))?;
            for line in &self.lines {
                writeln!(handle, "{}", line.raw())
                    .with_context(|| format!("could not write {}", temp.display()))?;
            }
            handle.sync_all().with_context(|| format!("could not flush {}", temp.display()))?;
        }

        fs::rename(&temp, &self.path)
            .with_context(|| format!("could not replace {}", self.path.display()))?;
        Ok(())
    }
}

/// Replace a line's declaration while leaving its comment and spacing alone.
fn rewrite(raw: &str, spec: &PackageSpec) -> String {
    let (declaration, comment) = grammar::split_comment(raw);
    match comment {
        // Reusing the whitespace that trailed the declaration keeps the
        // comment in its original column.
        Some(comment) => {
            let gap = &declaration[declaration.trim_end().len()..];
            format!("{spec}{gap}{comment}")
        }
        None => spec.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!("mpm-test-{tag}-{unique}"));
            fs::create_dir_all(&path).expect("temp dir");
            Self(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const SAMPLE: &str = "\
# editors
vim
neovim          # the good one

# search
ripgrep 14.1.0  # locked
";

    #[test]
    fn missing_file_loads_as_empty() {
        let dir = TempDir::new("missing");
        let manifest = ManifestFile::load(&dir.join("pacman")).expect("load");
        assert_eq!(manifest.specs().count(), 0);
        assert!(!manifest.exists());
    }

    #[test]
    fn inline_comments_do_not_become_part_of_the_package() {
        let dir = TempDir::new("inline");
        let path = dir.join("pacman");
        fs::write(&path, SAMPLE).expect("seed");

        let manifest = ManifestFile::load(&path).expect("load");
        let names: Vec<&str> = manifest.specs().map(|spec| spec.name.as_str()).collect();
        assert_eq!(names, vec!["vim", "neovim", "ripgrep"]);
    }

    #[test]
    fn comments_and_blank_lines_survive_an_edit() {
        let dir = TempDir::new("preserve");
        let path = dir.join("pacman");
        fs::write(&path, SAMPLE).expect("seed");

        let mut manifest = ManifestFile::load(&path).expect("load");
        assert!(manifest.declare(&PackageSpec::new("fd")));
        manifest.save().expect("save");

        assert_eq!(fs::read_to_string(&path).expect("read back"), format!("{SAMPLE}fd\n"));
    }

    #[test]
    fn rewriting_a_line_keeps_its_trailing_comment() {
        let dir = TempDir::new("repin");
        let path = dir.join("pacman");
        fs::write(&path, SAMPLE).expect("seed");

        let mut manifest = ManifestFile::load(&path).expect("load");
        assert!(manifest.declare(&PackageSpec::pinned("ripgrep", "14.2.0")));
        manifest.save().expect("save");

        let written = fs::read_to_string(&path).expect("read back");
        assert!(written.contains("ripgrep 14.2.0  # locked"), "got: {written}");
        assert!(!written.contains("14.1.0"));

        manifest.declare(&PackageSpec::new("ripgrep"));
        manifest.save().expect("save");
        assert!(fs::read_to_string(&path).expect("read back").contains("ripgrep  # locked"));
    }

    #[test]
    fn declaring_an_existing_package_is_a_no_op() {
        let dir = TempDir::new("noop");
        let path = dir.join("pacman");
        fs::write(&path, SAMPLE).expect("seed");

        let mut manifest = ManifestFile::load(&path).expect("load");
        assert!(!manifest.declare(&PackageSpec::new("vim")));
    }

    #[test]
    fn undeclare_removes_the_whole_line_including_its_comment() {
        let dir = TempDir::new("undeclare");
        let path = dir.join("pacman");
        fs::write(&path, SAMPLE).expect("seed");

        let mut manifest = ManifestFile::load(&path).expect("load");
        assert!(manifest.undeclare("neovim"));
        assert!(!manifest.undeclare("neovim"));
        manifest.save().expect("save");

        let written = fs::read_to_string(&path).expect("read back");
        assert!(!written.contains("neovim"));
        assert!(!written.contains("the good one"));
        assert!(written.contains("vim"));
        assert!(written.contains("# editors"));
    }

    #[test]
    fn a_malformed_line_names_its_line_number() {
        let dir = TempDir::new("malformed");
        let path = dir.join("pacman");
        fs::write(&path, "vim\nripgrep 1.0 oops\n").expect("seed");

        let error = ManifestFile::load(&path).expect_err("must reject");
        assert!(format!("{error:#}").contains("pacman:2"));
    }

    #[test]
    fn saving_leaves_no_temporary_file_behind() {
        let dir = TempDir::new("atomic");
        let path = dir.join("pacman");
        let mut manifest = ManifestFile::load(&path).expect("load");
        manifest.declare(&PackageSpec::new("vim"));
        manifest.save().expect("save");

        let leftovers: Vec<_> = fs::read_dir(&dir.0)
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("mpm-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "left behind {leftovers:?}");
    }
}
