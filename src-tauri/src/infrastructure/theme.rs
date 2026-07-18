use std::fmt;
use std::sync::{Arc, RwLock};

use serde::Serialize;
use thiserror::Error;

use super::storage::{StorageError, StorageService};

const THEME_MODE_KEY: &str = "theme.mode";
const THEME_ACCENT_KEY: &str = "theme.accent";
const THEME_ANIMATION_SPEED_KEY: &str = "theme.animation_speed";
const THEME_REDUCE_TRANSPARENCY_KEY: &str = "theme.reduce_transparency";
const THEME_REVISION_KEY: &str = "theme.revision";
const DEFAULT_ACCENT: &str = "#216bd9";
const ALLOWED_ACCENTS: [&str; 6] = [
    "#216bd9", "#7955c7", "#008b83", "#c7427a", "#e36a00", "#6d7782",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub fn parse(value: &str) -> Result<Self, ThemeValidationError> {
        match value {
            "system" => Ok(Self::System),
            "light" => Ok(Self::Light),
            "dark" => Ok(Self::Dark),
            _ => Err(ThemeValidationError::InvalidThemeMode(value.to_owned())),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AnimationSpeed {
    Slow,
    Normal,
    Fast,
}

impl AnimationSpeed {
    pub fn parse(value: &str) -> Result<Self, ThemeValidationError> {
        match value {
            "slow" => Ok(Self::Slow),
            "normal" => Ok(Self::Normal),
            "fast" => Ok(Self::Fast),
            _ => Err(ThemeValidationError::InvalidAnimationSpeed(
                value.to_owned(),
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Slow => "slow",
            Self::Normal => "normal",
            Self::Fast => "fast",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct AccentColor(String);

impl AccentColor {
    pub fn parse(value: &str) -> Result<Self, ThemeValidationError> {
        let canonical = value.to_ascii_lowercase();
        if !ALLOWED_ACCENTS.contains(&canonical.as_str()) {
            return Err(ThemeValidationError::InvalidAccent(value.to_owned()));
        }
        Ok(Self(canonical))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for AccentColor {
    fn default() -> Self {
        Self(DEFAULT_ACCENT.to_owned())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemePreferences {
    pub mode: ThemeMode,
    pub accent: AccentColor,
    pub animation_speed: AnimationSpeed,
    pub reduce_transparency: bool,
}

impl Default for ThemePreferences {
    fn default() -> Self {
        Self {
            mode: ThemeMode::Light,
            accent: AccentColor::default(),
            animation_speed: AnimationSpeed::Normal,
            reduce_transparency: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ThemeSnapshot {
    #[serde(flatten)]
    pub preferences: ThemePreferences,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThemePreferencesPatch {
    pub mode: Option<ThemeMode>,
    pub accent: Option<AccentColor>,
    pub animation_speed: Option<AnimationSpeed>,
    pub reduce_transparency: Option<bool>,
}

impl ThemePreferencesPatch {
    pub fn is_empty(&self) -> bool {
        self.mode.is_none()
            && self.accent.is_none()
            && self.animation_speed.is_none()
            && self.reduce_transparency.is_none()
    }

    fn apply_to(self, current: &ThemePreferences) -> ThemePreferences {
        ThemePreferences {
            mode: self.mode.unwrap_or(current.mode),
            accent: self.accent.unwrap_or_else(|| current.accent.clone()),
            animation_speed: self.animation_speed.unwrap_or(current.animation_speed),
            reduce_transparency: self
                .reduce_transparency
                .unwrap_or(current.reduce_transparency),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ThemeValidationError {
    #[error("unsupported theme mode: {0}")]
    InvalidThemeMode(String),
    #[error("unsupported accent color: {0}")]
    InvalidAccent(String),
    #[error("unsupported animation speed: {0}")]
    InvalidAnimationSpeed(String),
    #[error("theme update must include at least one field")]
    EmptyPatch,
}

#[derive(Debug, Error)]
pub enum ThemeError {
    #[error("theme storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("corrupt theme setting {key}: {reason}")]
    CorruptSettings { key: &'static str, reason: String },
    #[error("theme revision overflow")]
    RevisionOverflow,
    #[error("theme update patch is empty")]
    EmptyPatch,
    #[error("theme revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("theme state lock is poisoned")]
    StateLockPoisoned,
}

trait ThemeSettingsStore: fmt::Debug + Send + Sync {
    fn read_setting(&self, key: &str) -> Result<Option<String>, StorageError>;
    fn write_settings(&self, settings: &[(&str, &str)]) -> Result<(), StorageError>;
}

impl ThemeSettingsStore for StorageService {
    fn read_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        StorageService::read_setting(self, key)
    }

    fn write_settings(&self, settings: &[(&str, &str)]) -> Result<(), StorageError> {
        StorageService::write_settings(self, settings)
    }
}

#[derive(Debug)]
pub struct ThemeService {
    store: Arc<dyn ThemeSettingsStore>,
    state: RwLock<ThemeSnapshot>,
}

impl ThemeService {
    pub fn initialize(storage: Arc<StorageService>) -> Result<Self, ThemeError> {
        Self::initialize_with_store(storage)
    }

    fn initialize_with_store(store: Arc<dyn ThemeSettingsStore>) -> Result<Self, ThemeError> {
        let (snapshot, has_missing_values) = load_snapshot(store.as_ref())?;
        if has_missing_values {
            persist_snapshot(store.as_ref(), &snapshot)?;
        }
        Ok(Self {
            store,
            state: RwLock::new(snapshot),
        })
    }

    pub fn current(&self) -> Result<ThemeSnapshot, ThemeError> {
        self.state
            .read()
            .map(|snapshot| snapshot.clone())
            .map_err(|_| ThemeError::StateLockPoisoned)
    }

    pub fn update(
        &self,
        expected_revision: u64,
        patch: ThemePreferencesPatch,
    ) -> Result<ThemeSnapshot, ThemeError> {
        if patch.is_empty() {
            return Err(ThemeError::EmptyPatch);
        }
        let mut current = self
            .state
            .write()
            .map_err(|_| ThemeError::StateLockPoisoned)?;
        if current.revision != expected_revision {
            return Err(ThemeError::RevisionConflict {
                expected: expected_revision,
                actual: current.revision,
            });
        }
        let revision = current
            .revision
            .checked_add(1)
            .ok_or(ThemeError::RevisionOverflow)?;
        let next = ThemeSnapshot {
            preferences: patch.apply_to(&current.preferences),
            revision,
        };

        persist_snapshot(self.store.as_ref(), &next)?;
        *current = next.clone();
        Ok(next)
    }
}

fn persist_snapshot(
    store: &dyn ThemeSettingsStore,
    snapshot: &ThemeSnapshot,
) -> Result<(), StorageError> {
    let reduce_transparency = if snapshot.preferences.reduce_transparency {
        "true"
    } else {
        "false"
    };
    let revision = snapshot.revision.to_string();
    store.write_settings(&[
        (THEME_MODE_KEY, snapshot.preferences.mode.as_str()),
        (THEME_ACCENT_KEY, snapshot.preferences.accent.as_str()),
        (
            THEME_ANIMATION_SPEED_KEY,
            snapshot.preferences.animation_speed.as_str(),
        ),
        (THEME_REDUCE_TRANSPARENCY_KEY, reduce_transparency),
        (THEME_REVISION_KEY, revision.as_str()),
    ])
}

fn load_snapshot(store: &dyn ThemeSettingsStore) -> Result<(ThemeSnapshot, bool), ThemeError> {
    let defaults = ThemeSnapshot::default();
    let (mode, mode_missing) = read_parsed_setting(
        store,
        THEME_MODE_KEY,
        defaults.preferences.mode,
        ThemeMode::parse,
    )?;
    let (accent, accent_missing) = read_parsed_setting(
        store,
        THEME_ACCENT_KEY,
        defaults.preferences.accent,
        AccentColor::parse,
    )?;
    let (animation_speed, speed_missing) = read_parsed_setting(
        store,
        THEME_ANIMATION_SPEED_KEY,
        defaults.preferences.animation_speed,
        AnimationSpeed::parse,
    )?;
    let (reduce_transparency, transparency_missing) =
        match store.read_setting(THEME_REDUCE_TRANSPARENCY_KEY)? {
            Some(value) if value == "true" => (true, false),
            Some(value) if value == "false" => (false, false),
            Some(value) => {
                return Err(corrupt(
                    THEME_REDUCE_TRANSPARENCY_KEY,
                    format!("invalid boolean value {value}"),
                ));
            }
            None => (defaults.preferences.reduce_transparency, true),
        };
    let (revision, revision_missing) = match store.read_setting(THEME_REVISION_KEY)? {
        Some(value) => (
            value
                .parse::<u64>()
                .map_err(|_| corrupt(THEME_REVISION_KEY, format!("invalid revision {value}")))?,
            false,
        ),
        None => (defaults.revision, true),
    };

    Ok((
        ThemeSnapshot {
            preferences: ThemePreferences {
                mode,
                accent,
                animation_speed,
                reduce_transparency,
            },
            revision,
        },
        mode_missing || accent_missing || speed_missing || transparency_missing || revision_missing,
    ))
}

fn read_parsed_setting<T, F>(
    store: &dyn ThemeSettingsStore,
    key: &'static str,
    default: T,
    parse: F,
) -> Result<(T, bool), ThemeError>
where
    F: FnOnce(&str) -> Result<T, ThemeValidationError>,
{
    match store.read_setting(key)? {
        Some(value) => parse(&value)
            .map(|parsed| (parsed, false))
            .map_err(|error| corrupt(key, error.to_string())),
        None => Ok((default, true)),
    }
}

fn corrupt(key: &'static str, reason: String) -> ThemeError {
    ThemeError::CorruptSettings { key, reason }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    use tempfile::tempdir;

    use super::*;

    #[derive(Debug, Default)]
    struct FailingStore {
        values: Mutex<HashMap<String, String>>,
        fail_writes: AtomicBool,
    }

    impl ThemeSettingsStore for FailingStore {
        fn read_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
            Ok(self.values.lock().unwrap().get(key).cloned())
        }

        fn write_settings(&self, settings: &[(&str, &str)]) -> Result<(), StorageError> {
            if self.fail_writes.load(Ordering::SeqCst) {
                return Err(StorageError::LockPoisoned);
            }
            let mut values = self.values.lock().unwrap();
            for (key, value) in settings {
                values.insert((*key).to_owned(), (*value).to_owned());
            }
            Ok(())
        }
    }

    fn custom_patch() -> ThemePreferencesPatch {
        ThemePreferencesPatch {
            mode: Some(ThemeMode::Dark),
            accent: Some(AccentColor::parse("#7955c7").unwrap()),
            animation_speed: Some(AnimationSpeed::Fast),
            reduce_transparency: Some(true),
        }
    }

    #[test]
    fn missing_settings_create_and_return_explicit_light_defaults() {
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());

        let service = ThemeService::initialize(Arc::clone(&storage)).unwrap();
        let snapshot = service.current().unwrap();

        assert_eq!(snapshot, ThemeSnapshot::default());
        assert_eq!(
            storage.read_setting(THEME_MODE_KEY).unwrap().unwrap(),
            "light"
        );
        assert_eq!(
            storage.read_setting(THEME_ACCENT_KEY).unwrap().unwrap(),
            DEFAULT_ACCENT
        );
        assert_eq!(
            storage.read_setting(THEME_REVISION_KEY).unwrap().unwrap(),
            "0"
        );
    }

    #[test]
    fn partial_patch_preserves_unspecified_fields_and_increments_revision() {
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        let service = ThemeService::initialize(storage).unwrap();

        let updated = service
            .update(
                0,
                ThemePreferencesPatch {
                    mode: Some(ThemeMode::Dark),
                    ..ThemePreferencesPatch::default()
                },
            )
            .unwrap();

        assert_eq!(updated.preferences.mode, ThemeMode::Dark);
        assert_eq!(updated.preferences.accent, AccentColor::default());
        assert_eq!(updated.preferences.animation_speed, AnimationSpeed::Normal);
        assert!(!updated.preferences.reduce_transparency);
        assert_eq!(updated.revision, 1);
    }

    #[test]
    fn update_persists_atomically_and_restart_restores_snapshot_and_revision() {
        let temp = tempdir().unwrap();
        let data_root = temp.path().join("app-data");
        let storage = Arc::new(StorageService::initialize(&data_root).unwrap());
        let service = ThemeService::initialize(Arc::clone(&storage)).unwrap();
        let expected = service.update(0, custom_patch()).unwrap();
        drop(service);
        drop(storage);

        let reopened_storage = Arc::new(StorageService::initialize(&data_root).unwrap());
        let reopened = ThemeService::initialize(reopened_storage).unwrap();
        assert_eq!(reopened.current().unwrap(), expected);
        assert_eq!(expected.revision, 1);
    }

    #[test]
    fn invalid_values_are_rejected_and_allowlisted_accent_is_canonicalized() {
        assert!(matches!(
            ThemeMode::parse("sepia"),
            Err(ThemeValidationError::InvalidThemeMode(_))
        ));
        assert!(matches!(
            AccentColor::parse("#ffffff"),
            Err(ThemeValidationError::InvalidAccent(_))
        ));
        assert!(matches!(
            AnimationSpeed::parse("instant"),
            Err(ThemeValidationError::InvalidAnimationSpeed(_))
        ));
        assert_eq!(AccentColor::parse("#C7427A").unwrap().as_str(), "#c7427a");
    }

    #[test]
    fn stale_revision_is_rejected_without_persistence_or_memory_change() {
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        let service = ThemeService::initialize(Arc::clone(&storage)).unwrap();
        let first = service.update(0, custom_patch()).unwrap();

        let error = service
            .update(
                0,
                ThemePreferencesPatch {
                    mode: Some(ThemeMode::System),
                    ..ThemePreferencesPatch::default()
                },
            )
            .unwrap_err();

        assert!(matches!(
            error,
            ThemeError::RevisionConflict {
                expected: 0,
                actual: 1
            }
        ));
        assert_eq!(service.current().unwrap(), first);
        assert_eq!(
            storage.read_setting(THEME_REVISION_KEY).unwrap().as_deref(),
            Some("1")
        );
    }

    #[test]
    fn empty_patch_is_rejected_without_advancing_memory_or_persisted_revision() {
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        let service = ThemeService::initialize(Arc::clone(&storage)).unwrap();
        let before = service.current().unwrap();

        let error = service
            .update(before.revision, ThemePreferencesPatch::default())
            .unwrap_err();

        assert!(matches!(error, ThemeError::EmptyPatch));
        assert_eq!(service.current().unwrap(), before);
        assert_eq!(
            storage.read_setting(THEME_REVISION_KEY).unwrap().as_deref(),
            Some("0")
        );
    }

    #[test]
    fn corrupt_persisted_value_returns_explicit_error_without_overwrite() {
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        storage
            .write_settings(&[(THEME_MODE_KEY, "sepia")])
            .unwrap();

        let error = ThemeService::initialize(Arc::clone(&storage)).unwrap_err();

        assert!(matches!(
            error,
            ThemeError::CorruptSettings {
                key: THEME_MODE_KEY,
                ..
            }
        ));
        assert_eq!(
            storage.read_setting(THEME_MODE_KEY).unwrap().unwrap(),
            "sepia"
        );
    }

    #[test]
    fn failed_persistence_does_not_change_in_memory_truth() {
        let store = Arc::new(FailingStore::default());
        let service = ThemeService::initialize_with_store(store.clone()).unwrap();
        let original = service.current().unwrap();
        let persisted_before = store.values.lock().unwrap().clone();
        store.fail_writes.store(true, Ordering::SeqCst);

        let error = service.update(0, custom_patch()).unwrap_err();

        assert!(matches!(error, ThemeError::Storage(_)));
        assert_eq!(service.current().unwrap(), original);
        assert_eq!(*store.values.lock().unwrap(), persisted_before);
    }
}
