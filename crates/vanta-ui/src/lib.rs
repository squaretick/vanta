//! `vanta-ui` — the branded terminal UI for the `vanta` CLI.
//!
//! Provides the ASCII wordmark [`banner`], a thin [`Progress`] handle over
//! [`indicatif`] for download bars and indeterminate spinners, and plain
//! [`step`] milestones. Everything is TTY- and `NO_COLOR`-aware: when stdout is
//! not a terminal or `NO_COLOR` is set, output degrades to terse, uncolored
//! status lines on stderr with no spinner/bar animation, keeping stdout clean
//! for scripts and shell hooks.
//!
//! All decorative output (banner, bars, spinners, status lines) is written to
//! **stderr**; callers keep machine-readable results on stdout themselves.
#![forbid(unsafe_code)]

use std::io::IsTerminal;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;

/// Brand accent, teal `#3DA38C`.
const TEAL: (u8, u8, u8) = (61, 163, 140);
/// Dimmer teal `#276F5F`, used for the bar's empty track and the tagline.
const TEAL_DIM: (u8, u8, u8) = (39, 111, 95);

/// Compact ASCII wordmark for `vanta`, rendered teal under the banner.
const WORDMARK: &str = r"
 __   ____ _ _ __ | |_ __ _
 \ \ / / _` | '_ \| __/ _` |
  \ V / (_| | | | | || (_| |
   \_/ \__,_|_| |_|\__\__,_|";

/// Whether decorative UI may animate (bars/spinners) and color may be emitted.
///
/// Plain mode (the inverse) is selected when stdout is not a terminal — piped,
/// redirected, or running under CI — or when the operator has set `NO_COLOR`
/// (<https://no-color.org/>). In plain mode we emit terse, uncolored status
/// lines and never draw a spinner or bar.
pub fn is_rich() -> bool {
    !no_color() && std::io::stdout().is_terminal()
}

/// Whether `NO_COLOR` is present (and non-empty) in the environment.
fn no_color() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty())
}

/// Print the branded wordmark banner once, at the top of an interactive
/// top-level command. Shows the teal wordmark, a dim tagline, and the version.
///
/// A no-op when not in rich mode, so it never pollutes scriptable output.
pub fn banner(version: &str) {
    if !is_rich() {
        return;
    }
    eprintln!("{}", WORDMARK.truecolor(TEAL.0, TEAL.1, TEAL.2));
    eprintln!(
        " {}  {}",
        "every developer tool, one command".truecolor(TEAL_DIM.0, TEAL_DIM.1, TEAL_DIM.2),
        format!("v{version}").truecolor(TEAL_DIM.0, TEAL_DIM.1, TEAL_DIM.2),
    );
    eprintln!();
}

/// Print a plain milestone line (terse status), prefixed with a teal `✓` in
/// rich mode. Always goes to stderr so stdout stays machine-readable.
pub fn step(msg: &str) {
    if is_rich() {
        eprintln!("{} {msg}", "✓".truecolor(TEAL.0, TEAL.1, TEAL.2));
    } else {
        eprintln!("✓ {msg}");
    }
}

/// Print a branded "running" header (teal `▸ running: <cmd>`) before a
/// subprocess inherits the terminal. Always emitted to stderr so the child's
/// stdout stays clean; uncolored in plain mode.
pub fn running(cmd: &str) {
    if is_rich() {
        eprintln!("{} {}", "▸ running:".truecolor(TEAL.0, TEAL.1, TEAL.2), cmd);
    } else {
        eprintln!("▸ running: {cmd}");
    }
}

/// A handle around an [`indicatif::ProgressBar`] for a single operation.
///
/// In plain mode the underlying bar is hidden (no animation, no per-tick
/// output); only the final [`finish_ok`](Progress::finish_ok) /
/// [`finish_err`](Progress::finish_err) status line is emitted, as terse
/// uncolored text on stderr.
pub struct Progress {
    bar: ProgressBar,
    rich: bool,
}

impl Progress {
    /// An indeterminate spinner with a message (e.g. verify/extract/bundle).
    pub fn new_spinner(msg: &str) -> Progress {
        let rich = is_rich();
        let bar = if rich {
            let pb = ProgressBar::new_spinner();
            pb.set_style(spinner_style());
            pb.enable_steady_tick(Duration::from_millis(90));
            pb.set_message(msg.to_string());
            pb
        } else {
            ProgressBar::hidden()
        };
        Progress { bar, rich }
    }

    /// A determinate byte-progress bar of `total` bytes (download). When the
    /// total is unknown it falls back to a byte-counting spinner.
    pub fn new_bar(msg: &str, total: Option<u64>) -> Progress {
        let rich = is_rich();
        let bar = if rich {
            match total {
                Some(n) => {
                    let pb = ProgressBar::new(n);
                    pb.set_style(bar_style());
                    pb.set_message(msg.to_string());
                    pb
                }
                None => {
                    let pb = ProgressBar::new_spinner();
                    pb.set_style(byte_spinner_style());
                    pb.enable_steady_tick(Duration::from_millis(90));
                    pb.set_message(msg.to_string());
                    pb
                }
            }
        } else {
            ProgressBar::hidden()
        };
        Progress { bar, rich }
    }

    /// Advance the bar by `n` units (bytes, for a download bar).
    pub fn inc(&self, n: u64) {
        self.bar.inc(n);
    }

    /// Replace the bar's message.
    pub fn set_msg(&self, msg: &str) {
        self.bar.set_message(msg.to_string());
    }

    /// Remove the bar/spinner from the screen without printing a status line.
    /// Used when swapping one phase's indicator for the next.
    pub fn clear(&self) {
        self.bar.finish_and_clear();
    }

    /// Finish the operation with a success line, prefixed by a teal `✓`.
    pub fn finish_ok(&self, msg: &str) {
        if self.rich {
            self.bar.finish_and_clear();
            eprintln!("{} {msg}", "✓".truecolor(TEAL.0, TEAL.1, TEAL.2));
        } else {
            self.bar.finish_and_clear();
            eprintln!("✓ {msg}");
        }
    }

    /// Finish the operation with a failure line, prefixed by a teal `✗`.
    pub fn finish_err(&self, msg: &str) {
        if self.rich {
            self.bar.finish_and_clear();
            eprintln!("{} {msg}", "✗".truecolor(TEAL.0, TEAL.1, TEAL.2));
        } else {
            self.bar.finish_and_clear();
            eprintln!("✗ {msg}");
        }
    }
}

/// Template for an indeterminate phase spinner.
fn spinner_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.cyan} {msg}")
        .unwrap()
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
}

/// Template for a byte-counting spinner (download of unknown length).
fn byte_spinner_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.cyan} {msg} {bytes} ({bytes_per_sec})")
        .unwrap()
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
}

/// Template for a determinate download bar: teal filled `█`, dim `░` track.
///
/// `indicatif` templates color via `console`'s dotted-style parser, which only
/// understands named or 256-color indices (no truecolor). 256-index `36`
/// (`#00AF87`) is the closest match to the brand teal `#3DA38C`; `23`
/// (`#005F5F`) approximates the dim track `#276F5F`.
fn bar_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "{spinner:.cyan} {msg} [{bar:24.36/23}] {bytes}/{total_bytes} ({eta})",
    )
    .unwrap()
    .progress_chars("█░")
    .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `NO_COLOR` (non-empty) forces plain mode regardless of the terminal.
    #[test]
    fn no_color_forces_plain_mode() {
        // Save and restore so we don't disturb other tests sharing the process.
        let prev = std::env::var_os("NO_COLOR");
        std::env::set_var("NO_COLOR", "1");
        assert!(no_color());
        assert!(!is_rich(), "NO_COLOR must disable rich mode");
        std::env::set_var("NO_COLOR", "");
        assert!(!no_color(), "empty NO_COLOR must not count as set");
        match prev {
            Some(v) => std::env::set_var("NO_COLOR", v),
            None => std::env::remove_var("NO_COLOR"),
        }
    }

    /// The progress styles must all parse (template + char widths valid).
    #[test]
    fn styles_parse() {
        let _ = spinner_style();
        let _ = byte_spinner_style();
        let _ = bar_style();
    }

    /// A hidden (plain-mode) bar accepts the full API without panicking and
    /// emits no draw output.
    #[test]
    fn hidden_bar_is_inert() {
        let p = Progress {
            bar: ProgressBar::hidden(),
            rich: false,
        };
        p.inc(10);
        p.set_msg("x");
        p.finish_ok("done");
    }
}
