use anyhow::{Context, Result, anyhow, bail};
use std::process::{Command, Stdio};

/// A fully-resolved command line.
///
/// Arguments are kept as a list and handed to the OS untouched. Nothing here is
/// ever re-parsed by a shell, so package names containing spaces, quotes or
/// glob characters cannot change the shape of the command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub program: String,
    pub args: Vec<String>,
    /// Escalate with `sudo` (or `$MPM_SUDO`) before running.
    pub needs_root: bool,
}

impl Invocation {
    pub fn new(program: &str) -> Self {
        Self {
            program: program.to_string(),
            args: Vec::new(),
            needs_root: false,
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn with_root(mut self) -> Self {
        self.needs_root = true;
        self
    }

    /// The program and arguments actually executed, after root escalation.
    ///
    /// When already running as root the elevator is still used if present;
    /// `sudo` is a no-op passthrough for root, so this needs no uid check.
    pub fn resolve(&self) -> (String, Vec<String>) {
        if self.needs_root
            && let Some(elevator) = elevator()
        {
            let mut args = Vec::with_capacity(self.args.len() + 1);
            args.push(self.program.clone());
            args.extend(self.args.iter().cloned());
            return (elevator, args);
        }
        (self.program.clone(), self.args.clone())
    }

    /// Run interactively, inheriting stdio so the package manager can prompt.
    pub fn run(&self) -> Result<()> {
        let (program, args) = self.resolve();
        let status = Command::new(&program)
            .args(&args)
            .status()
            .with_context(|| format!("could not launch `{program}`"))?;

        if !status.success() {
            match status.code() {
                Some(code) => bail!("`{}` exited with status {code}", self.display()),
                None => bail!("`{}` was killed by a signal", self.display()),
            }
        }
        Ok(())
    }

    /// Run non-interactively, returning everything it printed.
    ///
    /// The transcript comes back whether or not the command succeeded, so a
    /// failure can be shown in full rather than summarised away.
    pub fn run_captured(&self) -> (String, Result<()>) {
        let (program, args) = self.resolve();
        let output = match Command::new(&program)
            .args(&args)
            .stdin(Stdio::null())
            .output()
        {
            Ok(output) => output,
            Err(error) => {
                let reason = Err(error).with_context(|| format!("could not launch `{program}`"));
                return (String::new(), reason);
            }
        };

        let mut transcript = String::from_utf8_lossy(&output.stdout).into_owned();
        transcript.push_str(&String::from_utf8_lossy(&output.stderr));

        let result = if output.status.success() {
            Ok(())
        } else {
            Err(anyhow!(
                "`{}` failed: {}",
                self.display(),
                last_line(&output.stderr)
            ))
        };
        (transcript, result)
    }

    /// Output is decoded lossily: a package manager emitting stray non-UTF-8
    /// bytes should not abort the whole run.
    pub fn capture(&self) -> Result<String> {
        let (program, args) = self.resolve();
        let output = Command::new(&program)
            .args(&args)
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("could not launch `{program}`"))?;

        if !output.status.success() {
            bail!("`{}` failed: {}", self.display(), last_line(&output.stderr));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Human-readable rendering, quoted well enough to be pasted into a shell.
    pub fn display(&self) -> String {
        let (program, args) = self.resolve();
        let mut out = quote(&program);
        for arg in &args {
            out.push(' ');
            out.push_str(&quote(arg));
        }
        out
    }
}

/// The most specific line a failing command wrote to stderr.
///
/// Tools put warnings first and the real complaint last, so without this the
/// reason a manager could not be read is thrown away.
fn last_line(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    match text.lines().map(str::trim).rfind(|line| !line.is_empty()) {
        Some(line) => line.to_string(),
        None => "no output on stderr".to_string(),
    }
}

/// `MPM_SUDO` selects an alternative (`doas`, `run0`); setting it empty opts out
/// of escalation entirely.
fn elevator() -> Option<String> {
    let name = match std::env::var("MPM_SUDO") {
        Ok(value) => value.trim().to_string(),
        Err(_) => "sudo".to_string(),
    };
    if name.is_empty() {
        return None;
    }
    which::which(&name).ok().map(|_| name)
}

fn quote(text: &str) -> String {
    let safe = !text.is_empty()
        && text.chars().all(|c| {
            c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '=' | '@' | ':' | '+' | ',')
        });
    if safe {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_never_word_split() {
        let inv = Invocation::new("apt-get").arg("install").arg("weird name");
        assert_eq!(
            inv.args,
            vec!["install".to_string(), "weird name".to_string()]
        );
    }

    #[test]
    fn a_failure_reports_what_the_command_complained_about() {
        let inv = Invocation::new("sh").args(["-c", "echo boom >&2; exit 1"]);
        let error = inv.capture().expect_err("must fail");
        assert!(error.to_string().contains("boom"), "got: {error}");
    }

    #[test]
    fn a_silent_failure_says_so() {
        let inv = Invocation::new("sh").args(["-c", "exit 1"]);
        let error = inv.capture().expect_err("must fail");
        assert!(error.to_string().contains("no output on stderr"));
    }

    #[test]
    fn capture_returns_stdout_on_success() {
        let inv = Invocation::new("printf").arg("vim 9.1\n");
        assert_eq!(inv.capture().expect("succeeds"), "vim 9.1\n");
    }

    #[test]
    fn display_quotes_unsafe_arguments() {
        let inv = Invocation::new("apt-get").arg("install").arg("weird name");
        assert_eq!(inv.display(), "apt-get install 'weird name'");
    }

    #[test]
    fn display_leaves_ordinary_arguments_bare() {
        let inv = Invocation::new("cargo").args(["install", "ripgrep", "--version", "14.1.0"]);
        assert_eq!(inv.display(), "cargo install ripgrep --version 14.1.0");
    }

    #[test]
    fn empty_mpm_sudo_disables_escalation() {
        temp_env("MPM_SUDO", "", || {
            let inv = Invocation::new("pacman").arg("-S").with_root();
            assert_eq!(inv.resolve().0, "pacman");
        });
    }

    fn temp_env(key: &str, value: &str, body: impl FnOnce()) {
        let previous = std::env::var(key).ok();
        unsafe { std::env::set_var(key, value) };
        body();
        match previous {
            Some(value) => unsafe { std::env::set_var(key, value) },
            None => unsafe { std::env::remove_var(key) },
        }
    }
}
