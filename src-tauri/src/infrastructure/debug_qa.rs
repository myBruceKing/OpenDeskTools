use std::ffi::OsString;
use std::time::Duration;

use thiserror::Error;

const OPEN_CLIPBOARD_SURFACE_AFTER_MS: &str = "--qa-open-clipboard-surface-after-ms";
const SCREENSHOT_PROBE: &str = "--qa-screenshot-probe";
const MAX_QA_DELAY_MS: u64 = 300_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DebugQaOptions {
    pub open_clipboard_surface_after: Option<Duration>,
    pub screenshot_probe: bool,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DebugQaArgumentError {
    #[error("QA arguments are available only in debug builds")]
    UnavailableInRelease,
    #[error("{OPEN_CLIPBOARD_SURFACE_AFTER_MS} requires a millisecond value")]
    MissingDelay,
    #[error("{OPEN_CLIPBOARD_SURFACE_AFTER_MS} may only be provided once")]
    DuplicateDelay,
    #[error("{SCREENSHOT_PROBE} may only be provided once")]
    DuplicateScreenshotProbe,
    #[error(
        "{OPEN_CLIPBOARD_SURFACE_AFTER_MS} must be an integer from 0 through {MAX_QA_DELAY_MS}"
    )]
    InvalidDelay,
}

pub fn parse(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<DebugQaOptions, DebugQaArgumentError> {
    parse_with_build_mode(arguments, cfg!(debug_assertions))
}

fn parse_with_build_mode(
    arguments: impl IntoIterator<Item = OsString>,
    debug_enabled: bool,
) -> Result<DebugQaOptions, DebugQaArgumentError> {
    let mut arguments = arguments.into_iter();
    let _executable = arguments.next();
    let mut delay = None;
    let mut screenshot_probe = false;
    while let Some(argument) = arguments.next() {
        if argument == SCREENSHOT_PROBE {
            if !debug_enabled {
                return Err(DebugQaArgumentError::UnavailableInRelease);
            }
            if screenshot_probe {
                return Err(DebugQaArgumentError::DuplicateScreenshotProbe);
            }
            screenshot_probe = true;
            continue;
        }
        let value = if argument == OPEN_CLIPBOARD_SURFACE_AFTER_MS {
            Some(arguments.next().ok_or(DebugQaArgumentError::MissingDelay)?)
        } else {
            argument
                .to_str()
                .and_then(|value| {
                    value.strip_prefix(&format!("{OPEN_CLIPBOARD_SURFACE_AFTER_MS}="))
                })
                .map(OsString::from)
        };
        let Some(value) = value else {
            continue;
        };
        if !debug_enabled {
            return Err(DebugQaArgumentError::UnavailableInRelease);
        }
        if delay.is_some() {
            return Err(DebugQaArgumentError::DuplicateDelay);
        }
        let value = value
            .to_str()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value <= MAX_QA_DELAY_MS)
            .ok_or(DebugQaArgumentError::InvalidDelay)?;
        delay = Some(Duration::from_millis(value));
    }
    Ok(DebugQaOptions {
        open_clipboard_surface_after: delay,
        screenshot_probe,
    })
}

#[cfg(debug_assertions)]
pub fn trace(message: impl AsRef<str>) {
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TRACE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let line = format!("{timestamp_ms} {}\n", message.as_ref());
    eprint!("[clipboard-surface-qa] {line}");
    let Ok(_guard) = TRACE_LOCK.get_or_init(|| Mutex::new(())).lock() else {
        return;
    };
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(trace_path())
    {
        let _ = file.write_all(line.as_bytes());
        let _ = file.flush();
    }
}

#[cfg(not(debug_assertions))]
pub fn trace(_message: impl AsRef<str>) {}

#[cfg(debug_assertions)]
pub fn trace_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "OpenDeskTools-clipboard-surface-qa-{}.log",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        std::iter::once(OsString::from("open-desk-tools.exe"))
            .chain(values.iter().map(OsString::from))
            .collect()
    }

    #[test]
    fn debug_build_accepts_separate_and_equals_delay_forms() {
        assert_eq!(
            parse_with_build_mode(args(&[OPEN_CLIPBOARD_SURFACE_AFTER_MS, "125"]), true)
                .unwrap()
                .open_clipboard_surface_after,
            Some(Duration::from_millis(125))
        );
        assert_eq!(
            parse_with_build_mode(args(&["--qa-open-clipboard-surface-after-ms=0"]), true)
                .unwrap()
                .open_clipboard_surface_after,
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn screenshot_probe_is_explicit_debug_only_and_unique() {
        assert!(
            parse_with_build_mode(args(&[SCREENSHOT_PROBE]), true)
                .unwrap()
                .screenshot_probe
        );
        assert_eq!(
            parse_with_build_mode(args(&[SCREENSHOT_PROBE]), false),
            Err(DebugQaArgumentError::UnavailableInRelease)
        );
        assert_eq!(
            parse_with_build_mode(args(&[SCREENSHOT_PROBE, SCREENSHOT_PROBE]), true),
            Err(DebugQaArgumentError::DuplicateScreenshotProbe)
        );
    }

    #[test]
    fn release_build_rejects_the_qa_argument() {
        assert_eq!(
            parse_with_build_mode(args(&[OPEN_CLIPBOARD_SURFACE_AFTER_MS, "10"]), false),
            Err(DebugQaArgumentError::UnavailableInRelease)
        );
        assert_eq!(
            parse_with_build_mode(args(&["--qa-open-clipboard-surface-after-ms=10"]), false),
            Err(DebugQaArgumentError::UnavailableInRelease)
        );
    }

    #[test]
    fn debug_delay_rejects_missing_duplicate_invalid_and_excessive_values() {
        assert_eq!(
            parse_with_build_mode(args(&[OPEN_CLIPBOARD_SURFACE_AFTER_MS]), true),
            Err(DebugQaArgumentError::MissingDelay)
        );
        assert_eq!(
            parse_with_build_mode(
                args(&[
                    OPEN_CLIPBOARD_SURFACE_AFTER_MS,
                    "1",
                    OPEN_CLIPBOARD_SURFACE_AFTER_MS,
                    "2"
                ]),
                true
            ),
            Err(DebugQaArgumentError::DuplicateDelay)
        );
        for value in ["-1", "abc", "300001"] {
            assert_eq!(
                parse_with_build_mode(args(&[OPEN_CLIPBOARD_SURFACE_AFTER_MS, value]), true),
                Err(DebugQaArgumentError::InvalidDelay)
            );
        }
    }
}
