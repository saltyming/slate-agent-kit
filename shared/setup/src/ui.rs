//! The terminal experience: output, colour, steps, menus and prompts.
//!
//! Owns where text goes (stdout for reports, the controlling terminal for
//! questions), when colour and symbols are used (a terminal with `NO_COLOR`
//! unset), and the rules that questions show their allowed values, reject
//! invalid answers and never wait when there is no terminal or `--yes`. It does
//! not know what the questions mean; `wizard` builds them.
//!
//! Main entry points: [`Ui`], [`Question`], [`Style`] and [`Sink`].

use std::io::{self, BufRead, IsTerminal, Write};
use std::sync::{Arc, Mutex};

/// Text style for [`Ui::paint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Bold text.
    Bold,
    /// Dimmed text.
    Dim,
    /// Green text.
    Green,
    /// Yellow text.
    Yellow,
    /// Red text.
    Red,
    /// Cyan text.
    Cyan,
}

/// A shared byte buffer, used to capture output in tests.
pub type Sink = Arc<Mutex<Vec<u8>>>;

struct SinkWriter(Sink);

impl Write for SinkWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Ok(mut v) = self.0.lock() {
            v.extend_from_slice(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// One question with its allowed values.
pub struct Question<'a> {
    /// The question text.
    pub prompt: String,
    /// A line of context shown above the menu.
    pub help: Option<String>,
    /// Menu entries as `(value, description)`.
    pub options: Vec<(String, String)>,
    /// The value taken by an empty answer.
    pub default: String,
    /// Whether the user may type a value that is not in the menu.
    pub allow_custom: bool,
    /// Whether `-` clears the value to blank.
    pub allow_clear: bool,
    /// Checks a typed value and returns it normalised, or a message naming the allowed values.
    pub validate: &'a dyn Fn(&str) -> Result<String, String>,
}

/// Terminal output and input.
pub struct Ui {
    out: Box<dyn Write>,
    input: Option<Box<dyn BufRead>>,
    prompt_out: Option<Box<dyn Write>>,
    color: bool,
    assume_yes: bool,
}

impl Ui {
    /// The interface for the running process.
    ///
    /// Questions go to the controlling terminal (`/dev/tty`, or `CONIN$` and
    /// `CONOUT$` on Windows) even when standard input is a pipe. With `assume_yes`
    /// no terminal is opened and every question takes its default.
    pub fn for_process(assume_yes: bool, no_color: bool) -> Ui {
        let stdout_tty = io::stdout().is_terminal();
        let color = stdout_tty && !no_color && enable_ansi();
        let (input, prompt_out) = if assume_yes {
            (None, None)
        } else {
            match open_terminal() {
                Some((i, o)) => (Some(i), Some(o)),
                None => (None, None),
            }
        };
        Ui {
            out: Box::new(io::stdout()),
            input,
            prompt_out,
            color,
            assume_yes,
        }
    }

    /// A non-interactive interface whose output goes to `sink`.
    pub fn captured(sink: Sink) -> Ui {
        Ui {
            out: Box::new(SinkWriter(sink)),
            input: None,
            prompt_out: None,
            color: false,
            assume_yes: true,
        }
    }

    /// An interactive interface that reads `answers` (one per line) and writes to `sink`.
    pub fn scripted(answers: &str, sink: Sink) -> Ui {
        Ui {
            out: Box::new(SinkWriter(sink.clone())),
            input: Some(Box::new(io::Cursor::new(answers.to_string().into_bytes()))),
            prompt_out: Some(Box::new(SinkWriter(sink))),
            color: false,
            assume_yes: false,
        }
    }

    /// True when questions are asked; false with `--yes` or without a terminal.
    pub fn interactive(&self) -> bool {
        !self.assume_yes && self.input.is_some()
    }

    /// True when colour and symbols are in use.
    pub fn color(&self) -> bool {
        self.color
    }

    /// Wraps `text` in the ANSI sequence of `style` when colour is on.
    pub fn paint(&self, style: Style, text: &str) -> String {
        if !self.color {
            return text.to_string();
        }
        let code = match style {
            Style::Bold => "1",
            Style::Dim => "2",
            Style::Green => "32",
            Style::Yellow => "33",
            Style::Red => "31",
            Style::Cyan => "36",
        };
        format!("\x1b[{code}m{text}\x1b[0m")
    }

    /// Prints one line to standard output.
    pub fn line(&mut self, text: &str) {
        let _ = writeln!(self.out, "{text}");
        let _ = self.out.flush();
    }

    /// Prints an empty line.
    pub fn blank(&mut self) {
        self.line("");
    }

    /// Prints a section title.
    pub fn title(&mut self, text: &str) {
        let t = self.paint(Style::Bold, text);
        self.line(&t);
    }

    /// Prints a numbered step, such as `[2/6] MCP servers`.
    pub fn step(&mut self, n: usize, total: usize, title: &str) {
        self.blank();
        let head = self.paint(Style::Bold, &format!("[{n}/{total}] {title}"));
        self.line(&head);
    }

    /// Prints a success line.
    pub fn ok(&mut self, text: &str) {
        let mark = if self.color {
            self.paint(Style::Green, "✓")
        } else {
            "ok".to_string()
        };
        self.line(&format!("  {mark} {text}"));
    }

    /// Prints a warning line.
    pub fn warn(&mut self, text: &str) {
        let mark = if self.color {
            self.paint(Style::Yellow, "!")
        } else {
            "warning:".to_string()
        };
        self.line(&format!("  {mark} {text}"));
    }

    /// Prints a failure line.
    pub fn fail(&mut self, text: &str) {
        let mark = if self.color {
            self.paint(Style::Red, "✗")
        } else {
            "error:".to_string()
        };
        self.line(&format!("  {mark} {text}"));
    }

    /// Prints a dimmed detail line.
    pub fn detail(&mut self, text: &str) {
        let t = self.paint(Style::Dim, text);
        self.line(&format!("    {t}"));
    }

    fn ask_out(&mut self, text: &str) {
        let w: &mut dyn Write = match self.prompt_out.as_mut() {
            Some(w) => w,
            None => &mut self.out,
        };
        let _ = write!(w, "{text}");
        let _ = w.flush();
    }

    fn ask_line(&mut self, text: &str) {
        self.ask_out(&format!("{text}\n"));
    }

    fn read_answer(&mut self) -> Option<String> {
        let input = self.input.as_mut()?;
        let mut buf = String::new();
        match input.read_line(&mut buf) {
            Ok(0) | Err(_) => {
                // The terminal went away; from here on every question takes its default.
                self.input = None;
                None
            }
            Ok(_) => Some(buf.trim().to_string()),
        }
    }

    /// Asks for free text. An empty answer takes `default`.
    pub fn ask_text(&mut self, prompt: &str, default: &str) -> String {
        if !self.interactive() {
            return default.to_string();
        }
        let shown = if default.is_empty() {
            String::new()
        } else {
            format!(" [{default}]")
        };
        self.ask_out(&format!("  {prompt}{shown}: "));
        match self.read_answer() {
            Some(a) if !a.is_empty() => a,
            _ => default.to_string(),
        }
    }

    /// Asks a yes/no question. Anything but `y`, `yes`, `n`, `no` or Enter is asked again.
    pub fn ask_yes_no(&mut self, prompt: &str, default: bool) -> bool {
        if !self.interactive() {
            return default;
        }
        let hint = if default { "Y/n" } else { "y/N" };
        loop {
            self.ask_out(&format!("  {prompt} [{hint}]: "));
            match self.read_answer() {
                None => return default,
                Some(a) => match a.to_ascii_lowercase().as_str() {
                    "" => return default,
                    "y" | "yes" => return true,
                    "n" | "no" => return false,
                    _ => self.ask_line("    Please answer y or n."),
                },
            }
        }
    }

    /// Asks a question with allowed values, asking again until the answer is valid.
    pub fn ask(&mut self, q: &Question<'_>) -> String {
        if !self.interactive() {
            return q.default.clone();
        }
        self.ask_out(&format!("  {}\n", q.prompt));
        if let Some(help) = &q.help {
            self.ask_line(&format!("    {help}"));
        }
        let width = q
            .options
            .iter()
            .map(|(v, _)| v.chars().count())
            .max()
            .unwrap_or(0);
        for (i, (value, desc)) in q.options.iter().enumerate() {
            let cur = if *value == q.default {
                "  (current)"
            } else {
                ""
            };
            let pad = " ".repeat(width - value.chars().count());
            self.ask_line(&format!("    {}) {value}{pad}  {desc}{cur}", i + 1));
        }
        let mut how = Vec::new();
        if !q.options.is_empty() {
            how.push(format!("1-{} or a value", q.options.len()));
        }
        if q.allow_custom && !q.options.is_empty() {
            how.push("or type your own".to_string());
        }
        if q.allow_clear {
            how.push("- clears".to_string());
        }
        let hint = if how.is_empty() {
            String::new()
        } else {
            format!(" ({})", how.join(", "))
        };
        let shown = if q.default.is_empty() {
            "blank".to_string()
        } else {
            q.default.clone()
        };
        loop {
            self.ask_out(&format!("    Choice [{shown}]{hint}: "));
            let Some(answer) = self.read_answer() else {
                return q.default.clone();
            };
            if answer.is_empty() {
                return q.default.clone();
            }
            if answer == "-" && q.allow_clear {
                return String::new();
            }
            let picked = answer
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=q.options.len()).contains(n))
                .map(|n| q.options[n - 1].0.clone())
                .or_else(|| {
                    q.options
                        .iter()
                        .find(|(v, _)| v.eq_ignore_ascii_case(&answer))
                        .map(|(v, _)| v.clone())
                });
            let candidate = match picked {
                Some(v) => v,
                None if q.allow_custom || q.options.is_empty() => answer.clone(),
                None => {
                    let allowed: Vec<&str> = q.options.iter().map(|(v, _)| v.as_str()).collect();
                    self.ask_line(&format!(
                        "    Not valid: `{answer}`. Choose 1-{} or one of: {}.",
                        q.options.len(),
                        allowed.join(", ")
                    ));
                    continue;
                }
            };
            match (q.validate)(&candidate) {
                Ok(v) => return v,
                Err(msg) => self.ask_line(&format!("    Not valid: {msg}.")),
            }
        }
    }

    /// Asks the confirmation before changes are applied. Without a terminal the answer is yes.
    pub fn confirm(&mut self, prompt: &str) -> bool {
        self.ask_yes_no(prompt, true)
    }
}

#[cfg(unix)]
fn open_terminal() -> Option<(Box<dyn BufRead>, Box<dyn Write>)> {
    let input = std::fs::File::open("/dev/tty").ok()?;
    let output = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/tty")
        .ok()?;
    Some((Box::new(io::BufReader::new(input)), Box::new(output)))
}

#[cfg(windows)]
fn open_terminal() -> Option<(Box<dyn BufRead>, Box<dyn Write>)> {
    let input = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONIN$")
        .ok()?;
    let output = std::fs::OpenOptions::new()
        .write(true)
        .open("CONOUT$")
        .ok()?;
    Some((Box::new(io::BufReader::new(input)), Box::new(output)))
}

#[cfg(not(any(unix, windows)))]
fn open_terminal() -> Option<(Box<dyn BufRead>, Box<dyn Write>)> {
    None
}

#[cfg(windows)]
fn enable_ansi() -> bool {
    use windows_sys::Win32::System::Console::{
        ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle, STD_OUTPUT_HANDLE,
        SetConsoleMode,
    };
    // SAFETY: the handle comes from GetStdHandle and the mode pointer is a live local.
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0u32;
        if GetConsoleMode(handle, &mut mode) == 0 {
            return false;
        }
        SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}

#[cfg(not(windows))]
fn enable_ansi() -> bool {
    true
}

/// Returns everything written to `sink` as text.
pub fn sink_text(sink: &Sink) -> String {
    sink.lock()
        .map(|v| String::from_utf8_lossy(&v).into_owned())
        .unwrap_or_default()
}

/// Creates an empty sink.
pub fn new_sink() -> Sink {
    Arc::new(Mutex::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level_question<'a>(validate: &'a dyn Fn(&str) -> Result<String, String>) -> Question<'a> {
        Question {
            prompt: "Level".into(),
            help: None,
            options: vec![
                ("on-request".into(), "only when you ask".into()),
                ("suggest".into(), "proposes".into()),
                ("auto".into(), "uses it".into()),
            ],
            default: "suggest".into(),
            allow_custom: false,
            allow_clear: false,
            validate,
        }
    }

    fn ok_validate(v: &str) -> Result<String, String> {
        Ok(v.to_string())
    }

    #[test]
    fn menu_answers_by_number_name_or_default() {
        let sink = new_sink();
        let mut ui = Ui::scripted("3\nON-REQUEST\n\n", sink.clone());
        let q = level_question(&ok_validate);
        assert_eq!(ui.ask(&q), "auto");
        assert_eq!(ui.ask(&q), "on-request");
        assert_eq!(ui.ask(&q), "suggest");
        let out = sink_text(&sink);
        assert!(out.contains("1) on-request"), "{out}");
        assert!(
            out.contains("2) suggest") && out.contains("(current)"),
            "{out}"
        );
    }

    #[test]
    fn invalid_answers_are_rejected_with_the_allowed_values_and_asked_again() {
        let sink = new_sink();
        let mut ui = Ui::scripted("9\nalways\n2\n", sink.clone());
        assert_eq!(ui.ask(&level_question(&ok_validate)), "suggest");
        let out = sink_text(&sink);
        assert_eq!(out.matches("Not valid").count(), 2, "{out}");
        assert!(out.contains("on-request, suggest, auto"), "{out}");
    }

    #[test]
    fn custom_values_go_through_the_validator() {
        let sink = new_sink();
        let mut ui = Ui::scripted("bad\ngood\n", sink.clone());
        let v = |s: &str| {
            if s == "good" {
                Ok("good".to_string())
            } else {
                Err("only `good`".to_string())
            }
        };
        let mut q = level_question(&v);
        q.allow_custom = true;
        assert_eq!(ui.ask(&q), "good");
        assert!(sink_text(&sink).contains("only `good`"));
    }

    #[test]
    fn dash_clears_only_when_allowed() {
        let sink = new_sink();
        let mut ui = Ui::scripted("-\n", sink);
        let mut q = level_question(&ok_validate);
        q.allow_clear = true;
        q.default = "auto".into();
        assert_eq!(ui.ask(&q), "");
    }

    #[test]
    fn yes_no_rejects_other_answers() {
        let sink = new_sink();
        let mut ui = Ui::scripted("maybe\nn\n\n", sink.clone());
        assert!(!ui.ask_yes_no("Go on?", true));
        assert!(ui.ask_yes_no("Again?", true));
        assert!(sink_text(&sink).contains("Please answer y or n"));
    }

    #[test]
    fn without_a_terminal_every_question_takes_its_default_and_prints_nothing() {
        let sink = new_sink();
        let mut ui = Ui::captured(sink.clone());
        assert!(!ui.interactive());
        assert_eq!(ui.ask(&level_question(&ok_validate)), "suggest");
        assert_eq!(ui.ask_text("Roots", "/x"), "/x");
        assert!(ui.ask_yes_no("Go?", true));
        assert!(!ui.ask_yes_no("Go?", false));
        assert_eq!(sink_text(&sink), "");
    }

    #[test]
    fn end_of_input_falls_back_to_defaults_for_the_rest_of_the_run() {
        let sink = new_sink();
        let mut ui = Ui::scripted("1\n", sink);
        assert_eq!(ui.ask(&level_question(&ok_validate)), "on-request");
        assert_eq!(ui.ask(&level_question(&ok_validate)), "suggest");
        assert!(!ui.interactive());
    }

    #[test]
    fn plain_output_has_no_escape_sequences() {
        let sink = new_sink();
        let mut ui = Ui::captured(sink.clone());
        ui.ok("done");
        ui.warn("careful");
        ui.fail("broken");
        ui.step(2, 6, "MCP servers");
        let out = sink_text(&sink);
        assert!(!out.contains('\x1b'));
        assert!(out.contains("[2/6] MCP servers"));
        assert!(out.contains("warning: careful") && out.contains("error: broken"));
    }
}
