//! Error type shared by every module of the installer.
//!
//! Owns the failure shape the terminal contract asks for: what failed, what
//! state it left, and the command that fixes or retries it. It does not decide
//! how errors are printed; `ui` and `lib` render them.
//!
//! Main entry points: [`Error`], [`Kind`], [`Result`] and [`IoContext`].

use std::fmt;
use std::io;

/// Class of failure; decides the process exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Bad command line or answer; nothing was attempted.
    Usage,
    /// A file system operation failed.
    Io,
    /// A configuration file could not be read or safely edited.
    Config,
    /// The kit payload or its descriptor is missing or malformed.
    Payload,
    /// A download failed or a checksum did not match.
    Network,
    /// A harness CLI or build command failed.
    Command,
    /// The user declined at the confirmation.
    Aborted,
}

/// A failure with an optional description of the state it left and a fix.
#[derive(Debug)]
pub struct Error {
    /// Class of the failure.
    pub kind: Kind,
    /// What failed.
    pub what: String,
    /// What state the failure left behind.
    pub state: Option<String>,
    /// The command or action that fixes or retries it.
    pub fix: Option<String>,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

/// Result alias used across the crate.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Creates an error of `kind` with the given description.
    pub fn new(kind: Kind, what: impl Into<String>) -> Self {
        Error {
            kind,
            what: what.into(),
            state: None,
            fix: None,
            source: None,
        }
    }

    /// A usage error (exit code 2).
    pub fn usage(what: impl Into<String>) -> Self {
        Self::new(Kind::Usage, what)
    }

    /// A configuration refusal or parse failure.
    pub fn config(what: impl Into<String>) -> Self {
        Self::new(Kind::Config, what)
    }

    /// A payload or descriptor problem.
    pub fn payload(what: impl Into<String>) -> Self {
        Self::new(Kind::Payload, what)
    }

    /// A download or checksum problem.
    pub fn network(what: impl Into<String>) -> Self {
        Self::new(Kind::Network, what)
    }

    /// A failed external command.
    pub fn command(what: impl Into<String>) -> Self {
        Self::new(Kind::Command, what)
    }

    /// An I/O failure with the operation that failed as context.
    pub fn io(context: impl Into<String>, source: io::Error) -> Self {
        let mut e = Self::new(Kind::Io, context);
        e.source = Some(Box::new(source));
        e
    }

    /// Records the state the failure left.
    pub fn with_state(mut self, state: impl Into<String>) -> Self {
        self.state = Some(state.into());
        self
    }

    /// Records the fix or retry command.
    pub fn with_fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }

    /// Process exit code for this failure.
    pub fn exit_code(&self) -> i32 {
        match self.kind {
            Kind::Usage => 2,
            _ => 1,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.what)?;
        if let Some(src) = &self.source {
            write!(f, ": {src}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

/// Adds a context message to an `io::Result`.
pub trait IoContext<T> {
    /// Converts the I/O error into an [`Error`] that names the failed operation.
    fn ctx(self, context: impl FnOnce() -> String) -> Result<T>;
}

impl<T> IoContext<T> for io::Result<T> {
    fn ctx(self, context: impl FnOnce() -> String) -> Result<T> {
        self.map_err(|e| Error::io(context(), e))
    }
}
