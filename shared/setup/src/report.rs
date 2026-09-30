//! What a run accomplished, and how it is shown at the end.
//!
//! Owns the [`Report`] that steps append to and its rendering: installed paths,
//! notes, commands the user has to run, problems that leave the run partly
//! done, and which harness to restart. It does not decide what counts as a problem.
//!
//! Main entry points: [`Report`] and [`Report::print`].

use crate::error::Error;
use crate::ui::Ui;

/// Accumulated results of a run.
#[derive(Debug, Default)]
pub struct Report {
    /// `(label, detail)` lines of what was installed, changed or removed.
    pub done: Vec<(String, String)>,
    /// Facts the user should know.
    pub notes: Vec<String>,
    /// Commands the installer could not run itself.
    pub manual: Vec<String>,
    /// Failures that left part of the run undone; they make the exit code non-zero.
    pub problems: Vec<Error>,
}

impl Report {
    /// Adds an `(label, detail)` line.
    pub fn done(&mut self, label: &str, detail: impl Into<String>) {
        self.done.push((label.to_string(), detail.into()));
    }

    /// Adds a note unless the same text is already there.
    pub fn note(&mut self, text: impl Into<String>) {
        let t = text.into();
        if !self.notes.contains(&t) {
            self.notes.push(t);
        }
    }

    /// Adds a command to run by hand unless it is already listed.
    pub fn manual(&mut self, cmd: impl Into<String>) {
        let c = cmd.into();
        if !self.manual.contains(&c) {
            self.manual.push(c);
        }
    }

    /// Prints the report. `restart` names the product to restart, if any.
    pub fn print(&self, ui: &mut Ui, headline: &str, restart: Option<&str>) {
        ui.blank();
        if self.problems.is_empty() {
            ui.title(headline);
        } else {
            ui.title(&format!("{headline} with problems"));
        }
        let width = self
            .done
            .iter()
            .map(|(l, _)| l.chars().count())
            .max()
            .unwrap_or(0);
        for (label, detail) in &self.done {
            let pad = " ".repeat(width - label.chars().count());
            ui.line(&format!("  {label}{pad}  {detail}"));
        }
        for n in &self.notes {
            ui.warn(n);
        }
        for p in &self.problems {
            ui.fail(&p.to_string());
            if let Some(state) = &p.state {
                ui.detail(&format!("state: {state}"));
            }
            if let Some(fix) = &p.fix {
                ui.detail(&format!("fix:   {fix}"));
            }
        }
        if !self.manual.is_empty() {
            ui.blank();
            ui.line("Run these commands yourself:");
            for c in &self.manual {
                ui.line(&format!("  {c}"));
            }
        }
        if let Some(product) = restart {
            ui.blank();
            ui.line(&format!("Restart {product} to load the changes."));
        }
    }
}
