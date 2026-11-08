//! Safe query and update operations for Arch based SigmaOS installations.
#![forbid(unsafe_code)]

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::process::Command;

/// A package upgrade reported by `pacman -Qu`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageUpdate {
    pub name: String,
    pub current_version: String,
    pub new_version: String,
}

/// A malformed line in pacman's upgrade listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line_number: usize,
    pub line: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid pacman upgrade line {}: {:?}",
            self.line_number, self.line
        )
    }
}

impl std::error::Error for ParseError {}

/// Parse the stable `name old-version -> new-version` form emitted by pacman.
pub fn parse_upgrade_list(output: &str) -> Result<Vec<PackageUpdate>, ParseError> {
    let mut updates = Vec::new();
    for (index, raw_line) in output.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let invalid = || ParseError {
            line_number: index + 1,
            line: line.to_owned(),
        };
        let (package_and_old, new_version) = line.split_once(" -> ").ok_or_else(invalid)?;
        let mut left = package_and_old.split_whitespace();
        let name = left.next().ok_or_else(invalid)?;
        let current_version = left.next().ok_or_else(invalid)?;
        if left.next().is_some()
            || new_version.is_empty()
            || new_version.chars().any(char::is_whitespace)
            || !name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || "@_+.-".contains(ch))
        {
            return Err(invalid());
        }
        updates.push(PackageUpdate {
            name: name.to_owned(),
            current_version: current_version.to_owned(),
            new_version: new_version.to_owned(),
        });
    }
    Ok(updates)
}

/// Output from a package-manager command, shaped for deterministic tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Default for CommandOutput {
    fn default() -> Self {
        Self {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
        }
    }
}

/// Small command boundary that keeps package policy testable and injectable.
pub trait CommandRunner {
    fn run(&self, program: &OsStr, args: &[&str], interactive: bool) -> io::Result<CommandOutput>;
}

/// Production runner using direct process execution without a shell.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, program: &OsStr, args: &[&str], interactive: bool) -> io::Result<CommandOutput> {
        let mut command = Command::new(program);
        command.args(args);
        if interactive {
            let status = command.status()?;
            Ok(CommandOutput {
                success: status.success(),
                exit_code: status.code(),
                stdout: String::new(),
                stderr: String::new(),
            })
        } else {
            let output = command.output()?;
            Ok(CommandOutput {
                success: output.status.success(),
                exit_code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            })
        }
    }
}

/// Errors returned while checking or applying package upgrades.
#[derive(Debug)]
pub enum UpdateError {
    Io(io::Error),
    CommandFailed {
        exit_code: Option<i32>,
        stderr: String,
    },
    InvalidOutput(ParseError),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "could not run pacman: {error}"),
            Self::CommandFailed { exit_code, stderr } => {
                write!(f, "pacman failed (exit {exit_code:?}): {}", stderr.trim())
            }
            Self::InvalidOutput(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for UpdateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::InvalidOutput(error) => Some(error),
            Self::CommandFailed { .. } => None,
        }
    }
}

/// Pacman update manager with an injectable command runner.
pub struct PacmanUpdateManager<R = SystemCommandRunner> {
    pacman: OsString,
    runner: R,
}

impl PacmanUpdateManager<SystemCommandRunner> {
    /// Use `pacman` from `PATH`.
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            pacman: program.into(),
            runner: SystemCommandRunner,
        }
    }
}

impl<R: CommandRunner> PacmanUpdateManager<R> {
    /// Construct a manager with a custom runner, useful for tests and integrations.
    pub fn with_runner(program: impl Into<OsString>, runner: R) -> Self {
        Self {
            pacman: program.into(),
            runner,
        }
    }

    /// Read available upgrades from the local sync databases. This does not refresh repositories.
    pub fn check_updates(&self) -> Result<Vec<PackageUpdate>, UpdateError> {
        let output = self.run_checked(&["-Qu"], false)?;
        parse_upgrade_list(&output.stdout).map_err(UpdateError::InvalidOutput)
    }

    /// Run a full system upgrade interactively. Callers must obtain user consent first.
    pub fn apply_upgrade(&self) -> Result<(), UpdateError> {
        self.run_checked(&["-Syu"], true).map(|_| ())
    }

    fn run_checked(&self, args: &[&str], interactive: bool) -> Result<CommandOutput, UpdateError> {
        let output = self
            .runner
            .run(&self.pacman, args, interactive)
            .map_err(UpdateError::Io)?;
        if output.success {
            Ok(output)
        } else {
            Err(UpdateError::CommandFailed {
                exit_code: output.exit_code,
                stderr: output.stderr,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeRunner {
        calls: RefCell<Vec<(OsString, Vec<String>, bool)>>,
        output: CommandOutput,
    }

    impl CommandRunner for FakeRunner {
        fn run(
            &self,
            program: &OsStr,
            args: &[&str],
            interactive: bool,
        ) -> io::Result<CommandOutput> {
            self.calls.borrow_mut().push((
                program.to_owned(),
                args.iter().map(|arg| (*arg).to_owned()).collect(),
                interactive,
            ));
            Ok(self.output.clone())
        }
    }

    #[test]
    fn parses_package_updates_and_preserves_version_epochs() {
        let updates = parse_upgrade_list("linux 6.8.1-1 -> 2:6.9-1\nfoo-bar 1.0 -> 1.1\n").unwrap();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].name, "linux");
        assert_eq!(updates[0].current_version, "6.8.1-1");
        assert_eq!(updates[0].new_version, "2:6.9-1");
    }

    #[test]
    fn rejects_malformed_package_output() {
        let error = parse_upgrade_list("not a pacman record").unwrap_err();
        assert_eq!(error.line_number, 1);
    }

    #[test]
    fn check_uses_local_query_without_interactive_mode() {
        let runner = FakeRunner {
            output: CommandOutput {
                success: true,
                exit_code: Some(0),
                stdout: "pkg 1 -> 2\n".into(),
                stderr: String::new(),
            },
            ..FakeRunner::default()
        };
        let manager = PacmanUpdateManager::with_runner("pacman", runner);

        let updates = manager.check_updates().unwrap();

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].name, "pkg");
        assert_eq!(manager.runner.calls.borrow()[0].1, vec!["-Qu"]);
        assert!(!manager.runner.calls.borrow()[0].2);
    }

    #[test]
    fn apply_uses_interactive_full_upgrade_command() {
        let runner = FakeRunner {
            output: CommandOutput {
                success: true,
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            },
            ..FakeRunner::default()
        };
        let manager = PacmanUpdateManager::with_runner("pacman", runner);

        manager.apply_upgrade().unwrap();

        assert_eq!(manager.runner.calls.borrow()[0].1, vec!["-Syu"]);
        assert!(manager.runner.calls.borrow()[0].2);
    }
}
