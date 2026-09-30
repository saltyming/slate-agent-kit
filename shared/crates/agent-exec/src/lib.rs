//! Runs one headless invocation of a backend CLI (codex, claude) for the
//! servers of this workspace and returns one record per run with the
//! backend's token usage normalized.
//!
//! The crate owns binary lookup ([`which`], [`version`], [`install_hint`]),
//! argv construction ([`command`]), spawning under the guard ([`guard`]),
//! output capture under a policy ([`CapturePolicy`]), cancellation, failure
//! classification ([`errkind`], [`failure_text`]) and the model fallback loop
//! ([`run`], [`run_with_fallback`]). A server decides what to run
//! ([`RunSpec`]) and what to do with the result: the crate writes no file and
//! no store, and reports only through [`RunEvent`] and its return value.
//! Session location and usage parsing live in `harness-log`; [`Usage`] and
//! [`UsageSource`] are re-exported from it.
//!
//! A guarded run starts the backend through `agent-guard`, a second executable
//! of this crate that is installed beside the servers ([`guard::locate`]); on
//! Windows through a Job Object instead. The crate never re-invokes the
//! running executable, so a program that links it needs no guard code in its
//! `main`. A server refuses nested requests with [`reentry::refused`].

mod argv;
mod capture;
mod discovery;
pub mod errkind;
mod failure;
pub mod guard;
pub mod reentry;
mod run;
mod spec;

pub use argv::command;
pub use capture::{Cap, CapturePolicy, Keep, Unit};
pub use discovery::{install_hint, version, which};
pub use errkind::BackendErrorKind;
pub use failure::{FailureTextPolicy, failure_text};
pub use harness_log::{Usage, UsageSource};
pub use run::{
    DiscardedAttempt, FallbackOutcome, Outcome, RunEvent, RunRecord, run, run_with_fallback,
};
pub use spec::{Backend, GuardMode, Isolation, OutputMode, Reentry, RunSpec, Sandbox};
