//! Stable error codes and the error type every tool returns.
//!
//! Owns the closed set of `error.code` values from the contract and the mapping of
//! I/O failures onto them. Does not format MCP results (see `server`).
//! Entry points: [`ErrCode`], [`PalError`], [`Res`].

use std::fmt;

/// The stable, machine-readable error categories of the contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrCode {
    /// An input is missing, malformed or not allowed.
    InvalidParams,
    /// No project root is configured, so the project cannot be checked against roots.
    NoProjectRoot,
    /// The project lies outside the project root and every extra root.
    OutsideRoots,
    /// The project has no `_palette/layout.rst`.
    NoLayout,
    /// `palette_init` found a layout already.
    AlreadyInitialized,
    /// A named file, record, item or entry does not exist.
    NotFound,
    /// A file the operation must change does not parse.
    ParseError,
    /// The per-project lock could not be taken in time.
    Locked,
    /// The result would violate a lint error or a lifecycle rule.
    InvariantViolation,
    /// A file changed between the read and the write.
    Conflict,
    /// The operating system refused a read or write.
    IoError,
}

impl ErrCode {
    /// The wire name of the code.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrCode::InvalidParams => "invalid_params",
            ErrCode::NoProjectRoot => "no_project_root",
            ErrCode::OutsideRoots => "outside_roots",
            ErrCode::NoLayout => "no_layout",
            ErrCode::AlreadyInitialized => "already_initialized",
            ErrCode::NotFound => "not_found",
            ErrCode::ParseError => "parse_error",
            ErrCode::Locked => "locked",
            ErrCode::InvariantViolation => "invariant_violation",
            ErrCode::Conflict => "conflict",
            ErrCode::IoError => "io_error",
        }
    }
}

/// An error with a stable code and a message that says how to fix it.
#[derive(Clone, Debug)]
pub struct PalError {
    /// The stable code.
    pub code: ErrCode,
    /// A human-readable message.
    pub message: String,
}

/// Result alias used across the crate.
pub type Res<T> = Result<T, PalError>;

impl PalError {
    /// Builds an error.
    pub fn new(code: ErrCode, message: impl Into<String>) -> PalError {
        PalError {
            code,
            message: message.into(),
        }
    }

    /// `invalid_params`.
    pub fn invalid(message: impl Into<String>) -> PalError {
        PalError::new(ErrCode::InvalidParams, message)
    }

    /// `not_found`.
    pub fn not_found(message: impl Into<String>) -> PalError {
        PalError::new(ErrCode::NotFound, message)
    }

    /// `invariant_violation`.
    pub fn invariant(message: impl Into<String>) -> PalError {
        PalError::new(ErrCode::InvariantViolation, message)
    }

    /// `parse_error` naming a file and a 1-based line.
    pub fn parse(file: &str, line: usize, message: impl Into<String>) -> PalError {
        PalError::new(
            ErrCode::ParseError,
            format!("{file}:{line}: {}", message.into()),
        )
    }

    /// `io_error` from an I/O error and the action that failed.
    pub fn io(action: &str, e: &std::io::Error) -> PalError {
        PalError::new(ErrCode::IoError, format!("{action}: {e}"))
    }
}

impl fmt::Display for PalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for PalError {}
