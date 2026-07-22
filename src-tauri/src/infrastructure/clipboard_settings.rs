//! Persisted clipboard preferences and their validation rules.

use regex::Regex;
use thiserror::Error;

use super::storage::{StorageError, StorageService};

const MONITORING_ENABLED_KEY: &str = "clipboard.monitoring_enabled";
const RETENTION_DAYS_KEY: &str = "clipboard.retention_days";
const MAX_ITEMS_KEY: &str = "clipboard.max_items";
const IGNORED_APPS_KEY: &str = "clipboard.ignored_apps";
const HISTORY_REUSE_STRATEGY_KEY: &str = "clipboard.history_reuse_strategy";
const SENSITIVE_RULES_KEY: &str = "clipboard.sensitive_rules";

pub const DEFAULT_MAX_ITEMS: u32 = 100;
pub const MIN_MAX_ITEMS: u32 = 10;
pub const MAX_MAX_ITEMS: u32 = 1_000;
pub const MAX_IGNORED_APPS: usize = 64;
pub const MAX_SENSITIVE_RULES: usize = 32;
pub const MAX_SENSITIVE_RULE_CHARS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardHistoryReuseStrategy {
    Promote,
    Keep,
}

impl ClipboardHistoryReuseStrategy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Promote => "promote",
            Self::Keep => "keep",
        }
    }

    fn parse(value: &str) -> Result<Self, ClipboardSettingsError> {
        match value {
            "promote" => Ok(Self::Promote),
            "keep" => Ok(Self::Keep),
            _ => Err(ClipboardSettingsError::InvalidHistoryReuseStrategy),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardSettings {
    pub retention_days: Option<u16>,
    pub max_items: u32,
    pub ignored_apps: Vec<String>,
    pub history_reuse_strategy: ClipboardHistoryReuseStrategy,
    pub sensitive_rules: Vec<String>,
}

impl Default for ClipboardSettings {
    fn default() -> Self {
        Self {
            retention_days: Some(30),
            max_items: DEFAULT_MAX_ITEMS,
            ignored_apps: Vec::new(),
            history_reuse_strategy: ClipboardHistoryReuseStrategy::Promote,
            sensitive_rules: Vec::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ClipboardSettingsError {
    #[error("clipboard settings storage operation failed")]
    Storage(#[from] StorageError),
    #[error("retention must be 7, 30, 90, 365 days, or unlimited")]
    InvalidRetentionDays,
    #[error("maximum history items is outside the supported range")]
    InvalidMaxItems,
    #[error("ignored applications are invalid")]
    InvalidIgnoredApps,
    #[error("history reuse strategy is invalid")]
    InvalidHistoryReuseStrategy,
    #[error("sensitive rules are invalid")]
    InvalidSensitiveRules,
}

pub fn monitoring_enabled(storage: &StorageService) -> Result<bool, StorageError> {
    Ok(storage
        .read_setting(MONITORING_ENABLED_KEY)?
        .map(|value| value == "true")
        .unwrap_or(true))
}

pub fn set_monitoring_enabled(storage: &StorageService, enabled: bool) -> Result<(), StorageError> {
    storage.write_settings(&[(
        MONITORING_ENABLED_KEY,
        if enabled { "true" } else { "false" },
    )])
}

pub fn load(storage: &StorageService) -> Result<ClipboardSettings, ClipboardSettingsError> {
    let defaults = ClipboardSettings::default();
    let retention_days = match storage.read_setting(RETENTION_DAYS_KEY)? {
        None => defaults.retention_days,
        Some(value) if value == "unlimited" => None,
        Some(value) => Some(
            value
                .parse::<u16>()
                .map_err(|_| ClipboardSettingsError::InvalidRetentionDays)?,
        ),
    };
    let max_items = storage
        .read_setting(MAX_ITEMS_KEY)?
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|_| ClipboardSettingsError::InvalidMaxItems)
        })
        .transpose()?
        .unwrap_or(defaults.max_items);
    let ignored_apps = storage
        .read_setting(IGNORED_APPS_KEY)?
        .map(|value| parse_ignored_apps(&value))
        .transpose()?
        .unwrap_or_default();
    let history_reuse_strategy = storage
        .read_setting(HISTORY_REUSE_STRATEGY_KEY)?
        .map(|value| ClipboardHistoryReuseStrategy::parse(&value))
        .transpose()?
        .unwrap_or(defaults.history_reuse_strategy);
    let sensitive_rules = storage
        .read_setting(SENSITIVE_RULES_KEY)?
        .map(|value| parse_sensitive_rules(&value))
        .transpose()?
        .unwrap_or_default();

    let settings = ClipboardSettings {
        retention_days,
        max_items,
        ignored_apps,
        history_reuse_strategy,
        sensitive_rules,
    };
    validate(&settings)?;
    Ok(settings)
}

pub fn save(
    storage: &StorageService,
    settings: &ClipboardSettings,
) -> Result<(), ClipboardSettingsError> {
    validate(settings)?;
    let retention = settings
        .retention_days
        .map(|days| days.to_string())
        .unwrap_or_else(|| "unlimited".to_owned());
    let max_items = settings.max_items.to_string();
    let ignored_apps = settings.ignored_apps.join("\n");
    let sensitive_rules = settings.sensitive_rules.join("\n");
    storage.write_settings(&[
        (RETENTION_DAYS_KEY, &retention),
        (MAX_ITEMS_KEY, &max_items),
        (IGNORED_APPS_KEY, &ignored_apps),
        (
            HISTORY_REUSE_STRATEGY_KEY,
            settings.history_reuse_strategy.as_str(),
        ),
        (SENSITIVE_RULES_KEY, &sensitive_rules),
    ])?;
    Ok(())
}

pub fn validate(settings: &ClipboardSettings) -> Result<(), ClipboardSettingsError> {
    if !matches!(settings.retention_days, None | Some(7 | 30 | 90 | 365)) {
        return Err(ClipboardSettingsError::InvalidRetentionDays);
    }
    if !(MIN_MAX_ITEMS..=MAX_MAX_ITEMS).contains(&settings.max_items) {
        return Err(ClipboardSettingsError::InvalidMaxItems);
    }
    if settings.ignored_apps.len() > MAX_IGNORED_APPS
        || settings
            .ignored_apps
            .iter()
            .any(|value| !is_safe_process_name(value))
    {
        return Err(ClipboardSettingsError::InvalidIgnoredApps);
    }
    if settings.sensitive_rules.len() > MAX_SENSITIVE_RULES
        || settings
            .sensitive_rules
            .iter()
            .any(|value| value.is_empty() || value.chars().count() > MAX_SENSITIVE_RULE_CHARS)
    {
        return Err(ClipboardSettingsError::InvalidSensitiveRules);
    }
    for rule in &settings.sensitive_rules {
        Regex::new(rule).map_err(|_| ClipboardSettingsError::InvalidSensitiveRules)?;
    }
    Ok(())
}

pub fn parse_ignored_apps(value: &str) -> Result<Vec<String>, ClipboardSettingsError> {
    let mut apps = value
        .split([',', '\n', '\r'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    apps.sort();
    apps.dedup();
    if apps.len() > MAX_IGNORED_APPS || apps.iter().any(|value| !is_safe_process_name(value)) {
        return Err(ClipboardSettingsError::InvalidIgnoredApps);
    }
    Ok(apps)
}

pub fn parse_sensitive_rules(value: &str) -> Result<Vec<String>, ClipboardSettingsError> {
    let rules = value
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let settings = ClipboardSettings {
        sensitive_rules: rules.clone(),
        ..ClipboardSettings::default()
    };
    validate(&settings)?;
    Ok(rules)
}

fn is_safe_process_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.ends_with(".exe")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn monitoring_defaults_to_enabled_and_persists() {
        let temp = tempdir().unwrap();
        let storage = StorageService::initialize(temp.path().join("data")).unwrap();
        assert!(monitoring_enabled(&storage).unwrap());
        set_monitoring_enabled(&storage, false).unwrap();
        assert!(!monitoring_enabled(&storage).unwrap());
    }

    #[test]
    fn settings_round_trip_with_normalized_processes_and_regexes() {
        let temp = tempdir().unwrap();
        let storage = StorageService::initialize(temp.path().join("data")).unwrap();
        let settings = ClipboardSettings {
            retention_days: Some(90),
            max_items: 300,
            ignored_apps: parse_ignored_apps("Notepad.EXE, chrome.exe").unwrap(),
            history_reuse_strategy: ClipboardHistoryReuseStrategy::Keep,
            sensitive_rules: parse_sensitive_rules("password\n\\btoken\\b").unwrap(),
        };
        save(&storage, &settings).unwrap();
        assert_eq!(load(&storage).unwrap(), settings);
    }
}
