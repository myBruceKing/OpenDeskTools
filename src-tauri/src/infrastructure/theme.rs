use std::fmt;
use std::path::Path;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::storage::{StorageError, StorageService};
use super::theme_asset::{ThemeAssetError, ThemeAssetService, ThemeBackgroundAsset};

const THEME_MODE_KEY: &str = "theme.mode";
const THEME_ACCENT_KEY: &str = "theme.accent";
const THEME_ANIMATION_SPEED_KEY: &str = "theme.animation_speed";
const THEME_REDUCE_TRANSPARENCY_KEY: &str = "theme.reduce_transparency";
const THEME_BACKGROUND_ASSET_KEY: &str = "theme.background_asset";
const THEME_BACKGROUND_FIT_KEY: &str = "theme.background_fit";
const THEME_BACKGROUND_DIM_KEY: &str = "theme.background_dim";
const THEME_BACKGROUND_BLUR_KEY: &str = "theme.background_blur";
const THEME_PANEL_OPACITY_KEY: &str = "theme.panel_opacity";
const THEME_REVISION_KEY: &str = "theme.revision";
const DEFAULT_ACCENT: &str = "#216bd9";
const DEFAULT_BACKGROUND_DIM: u8 = 24;
const DEFAULT_BACKGROUND_BLUR: u8 = 6;
const DEFAULT_PANEL_OPACITY: u8 = 86;
const MAX_BACKGROUND_DIM: u8 = 100;
const MAX_BACKGROUND_BLUR: u8 = 24;
const MIN_PANEL_OPACITY: u8 = 0;
const MAX_PANEL_OPACITY: u8 = 100;

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundFit {
    #[default]
    Cover,
    Contain,
}

impl BackgroundFit {
    pub fn parse(value: &str) -> Result<Self, ThemeValidationError> {
        match value {
            "cover" => Ok(Self::Cover),
            "contain" => Ok(Self::Contain),
            _ => Err(ThemeValidationError::InvalidBackgroundFit(value.to_owned())),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Cover => "cover",
            Self::Contain => "contain",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct AccentColor(String);

impl AccentColor {
    pub fn parse(value: &str) -> Result<Self, ThemeValidationError> {
        let canonical = value.to_ascii_lowercase();
        if canonical.len() != 7
            || !canonical.starts_with('#')
            || !canonical[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
        {
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
    pub background: Option<ThemeBackgroundAsset>,
    pub background_fit: BackgroundFit,
    pub background_dim: u8,
    pub background_blur: u8,
    pub panel_opacity: u8,
}

impl Default for ThemePreferences {
    fn default() -> Self {
        Self {
            mode: ThemeMode::Light,
            accent: AccentColor::default(),
            animation_speed: AnimationSpeed::Normal,
            reduce_transparency: false,
            background: None,
            background_fit: BackgroundFit::Cover,
            background_dim: DEFAULT_BACKGROUND_DIM,
            background_blur: DEFAULT_BACKGROUND_BLUR,
            panel_opacity: DEFAULT_PANEL_OPACITY,
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
    pub background: Option<Option<ThemeBackgroundAsset>>,
    pub background_fit: Option<BackgroundFit>,
    pub background_dim: Option<u8>,
    pub background_blur: Option<u8>,
    pub panel_opacity: Option<u8>,
}

impl ThemePreferencesPatch {
    pub fn is_empty(&self) -> bool {
        self.mode.is_none()
            && self.accent.is_none()
            && self.animation_speed.is_none()
            && self.reduce_transparency.is_none()
            && self.background.is_none()
            && self.background_fit.is_none()
            && self.background_dim.is_none()
            && self.background_blur.is_none()
            && self.panel_opacity.is_none()
    }

    pub fn validate(&self) -> Result<(), ThemeValidationError> {
        if self
            .background_dim
            .is_some_and(|value| value > MAX_BACKGROUND_DIM)
        {
            return Err(ThemeValidationError::InvalidBackgroundDim);
        }
        if self
            .background_blur
            .is_some_and(|value| value > MAX_BACKGROUND_BLUR)
        {
            return Err(ThemeValidationError::InvalidBackgroundBlur);
        }
        if self
            .panel_opacity
            .is_some_and(|value| !(MIN_PANEL_OPACITY..=MAX_PANEL_OPACITY).contains(&value))
        {
            return Err(ThemeValidationError::InvalidPanelOpacity);
        }
        if let Some(Some(asset)) = &self.background {
            asset
                .validate()
                .map_err(|_| ThemeValidationError::InvalidBackgroundAsset)?;
        }
        Ok(())
    }

    fn apply_to(self, current: &ThemePreferences) -> ThemePreferences {
        ThemePreferences {
            mode: self.mode.unwrap_or(current.mode),
            accent: self.accent.unwrap_or_else(|| current.accent.clone()),
            animation_speed: self.animation_speed.unwrap_or(current.animation_speed),
            reduce_transparency: self
                .reduce_transparency
                .unwrap_or(current.reduce_transparency),
            background: self
                .background
                .unwrap_or_else(|| current.background.clone()),
            background_fit: self.background_fit.unwrap_or(current.background_fit),
            background_dim: self.background_dim.unwrap_or(current.background_dim),
            background_blur: self.background_blur.unwrap_or(current.background_blur),
            panel_opacity: self.panel_opacity.unwrap_or(current.panel_opacity),
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
    #[error("unsupported background fit: {0}")]
    InvalidBackgroundFit(String),
    #[error("background asset metadata is invalid")]
    InvalidBackgroundAsset,
    #[error("background dim must be between 0 and {MAX_BACKGROUND_DIM}")]
    InvalidBackgroundDim,
    #[error("background blur must be between 0 and {MAX_BACKGROUND_BLUR}")]
    InvalidBackgroundBlur,
    #[error("panel opacity must be between {MIN_PANEL_OPACITY} and {MAX_PANEL_OPACITY}")]
    InvalidPanelOpacity,
    #[error("theme update must include at least one field")]
    EmptyPatch,
}

#[derive(Debug, Error)]
pub enum ThemeError {
    #[error("theme storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("theme background asset failed: {0}")]
    Asset(#[from] ThemeAssetError),
    #[error("theme preference serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("theme preference validation failed: {0}")]
    Validation(#[from] ThemeValidationError),
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
    assets: Option<ThemeAssetService>,
    state: RwLock<ThemeSnapshot>,
}

impl ThemeService {
    pub fn initialize(storage: Arc<StorageService>) -> Result<Self, ThemeError> {
        let assets = ThemeAssetService::initialize(&storage)?;
        Self::initialize_with_store_and_assets(storage, Some(assets))
    }

    #[cfg(test)]
    fn initialize_with_store(store: Arc<dyn ThemeSettingsStore>) -> Result<Self, ThemeError> {
        Self::initialize_with_store_and_assets(store, None)
    }

    fn initialize_with_store_and_assets(
        store: Arc<dyn ThemeSettingsStore>,
        assets: Option<ThemeAssetService>,
    ) -> Result<Self, ThemeError> {
        let (snapshot, has_missing_values) = load_snapshot(store.as_ref(), assets.as_ref())?;
        if has_missing_values {
            persist_snapshot(store.as_ref(), &snapshot)?;
        }
        if let Some(asset_service) = &assets {
            if let Err(error) = asset_service.reconcile(
                snapshot
                    .preferences
                    .background
                    .as_ref()
                    .map(|asset| asset.id.as_str()),
            ) {
                eprintln!("failed to reconcile theme background assets: {error}");
            }
        }
        Ok(Self {
            store,
            assets,
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
        patch.validate()?;
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

    pub fn import_background(
        &self,
        expected_revision: u64,
        source: &Path,
    ) -> Result<ThemeSnapshot, ThemeError> {
        {
            let current = self
                .state
                .read()
                .map_err(|_| ThemeError::StateLockPoisoned)?;
            if current.revision != expected_revision {
                return Err(ThemeError::RevisionConflict {
                    expected: expected_revision,
                    actual: current.revision,
                });
            }
        }
        let assets = self
            .assets
            .as_ref()
            .ok_or(ThemeError::Asset(ThemeAssetError::InvalidSource))?;
        let imported = assets.import(source)?;
        let previous_id = self
            .state
            .read()
            .map_err(|_| ThemeError::StateLockPoisoned)?
            .preferences
            .background
            .as_ref()
            .map(|asset| asset.id.clone());
        let new_id = imported.asset.id.clone();
        let updated = self.update(
            expected_revision,
            ThemePreferencesPatch {
                background: Some(Some(imported.asset)),
                ..ThemePreferencesPatch::default()
            },
        );
        if updated.is_err()
            && imported.newly_created
            && previous_id.as_deref() != Some(new_id.as_str())
        {
            let _ = assets.remove(&new_id);
        }
        let updated = updated?;
        if let Some(previous_id) = previous_id.filter(|id| id != &new_id) {
            if let Err(error) = assets.remove(&previous_id) {
                eprintln!("failed to remove replaced theme background: {error}");
            }
        }
        Ok(updated)
    }

    pub fn remove_background(&self, expected_revision: u64) -> Result<ThemeSnapshot, ThemeError> {
        let previous_id = self
            .state
            .read()
            .map_err(|_| ThemeError::StateLockPoisoned)?
            .preferences
            .background
            .as_ref()
            .map(|asset| asset.id.clone());
        let updated = self.update(
            expected_revision,
            ThemePreferencesPatch {
                background: Some(None),
                background_fit: Some(BackgroundFit::default()),
                background_dim: Some(DEFAULT_BACKGROUND_DIM),
                background_blur: Some(DEFAULT_BACKGROUND_BLUR),
                panel_opacity: Some(DEFAULT_PANEL_OPACITY),
                ..ThemePreferencesPatch::default()
            },
        )?;
        if let (Some(assets), Some(previous_id)) = (&self.assets, previous_id) {
            if let Err(error) = assets.remove(&previous_id) {
                eprintln!("failed to remove cleared theme background: {error}");
            }
        }
        Ok(updated)
    }

    pub fn read_background(&self) -> Result<Vec<u8>, ThemeError> {
        let background = self
            .state
            .read()
            .map_err(|_| ThemeError::StateLockPoisoned)?
            .preferences
            .background
            .clone()
            .ok_or(ThemeError::Asset(ThemeAssetError::Missing))?;
        self.assets
            .as_ref()
            .ok_or(ThemeError::Asset(ThemeAssetError::Missing))?
            .read(&background)
            .map_err(ThemeError::from)
    }
}

fn persist_snapshot(
    store: &dyn ThemeSettingsStore,
    snapshot: &ThemeSnapshot,
) -> Result<(), ThemeError> {
    let reduce_transparency = if snapshot.preferences.reduce_transparency {
        "true"
    } else {
        "false"
    };
    let background_asset = snapshot
        .preferences
        .background
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?
        .unwrap_or_default();
    let background_dim = snapshot.preferences.background_dim.to_string();
    let background_blur = snapshot.preferences.background_blur.to_string();
    let panel_opacity = snapshot.preferences.panel_opacity.to_string();
    let revision = snapshot.revision.to_string();
    store.write_settings(&[
        (THEME_MODE_KEY, snapshot.preferences.mode.as_str()),
        (THEME_ACCENT_KEY, snapshot.preferences.accent.as_str()),
        (
            THEME_ANIMATION_SPEED_KEY,
            snapshot.preferences.animation_speed.as_str(),
        ),
        (THEME_REDUCE_TRANSPARENCY_KEY, reduce_transparency),
        (THEME_BACKGROUND_ASSET_KEY, background_asset.as_str()),
        (
            THEME_BACKGROUND_FIT_KEY,
            snapshot.preferences.background_fit.as_str(),
        ),
        (THEME_BACKGROUND_DIM_KEY, background_dim.as_str()),
        (THEME_BACKGROUND_BLUR_KEY, background_blur.as_str()),
        (THEME_PANEL_OPACITY_KEY, panel_opacity.as_str()),
        (THEME_REVISION_KEY, revision.as_str()),
    ])?;
    Ok(())
}

fn load_snapshot(
    store: &dyn ThemeSettingsStore,
    assets: Option<&ThemeAssetService>,
) -> Result<(ThemeSnapshot, bool), ThemeError> {
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
    let (background, background_missing) =
        load_background_asset(store, assets, defaults.preferences.background.clone())?;
    let (background_fit, background_fit_missing) = read_parsed_setting(
        store,
        THEME_BACKGROUND_FIT_KEY,
        defaults.preferences.background_fit,
        BackgroundFit::parse,
    )?;
    let (background_dim, background_dim_missing) = read_u8_setting(
        store,
        THEME_BACKGROUND_DIM_KEY,
        defaults.preferences.background_dim,
        0,
        MAX_BACKGROUND_DIM,
    )?;
    let (background_blur, background_blur_missing) = read_u8_setting(
        store,
        THEME_BACKGROUND_BLUR_KEY,
        defaults.preferences.background_blur,
        0,
        MAX_BACKGROUND_BLUR,
    )?;
    let (panel_opacity, panel_opacity_missing) = read_u8_setting(
        store,
        THEME_PANEL_OPACITY_KEY,
        defaults.preferences.panel_opacity,
        MIN_PANEL_OPACITY,
        MAX_PANEL_OPACITY,
    )?;
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
                background,
                background_fit,
                background_dim,
                background_blur,
                panel_opacity,
            },
            revision,
        },
        mode_missing
            || accent_missing
            || speed_missing
            || transparency_missing
            || background_missing
            || background_fit_missing
            || background_dim_missing
            || background_blur_missing
            || panel_opacity_missing
            || revision_missing,
    ))
}

fn load_background_asset(
    store: &dyn ThemeSettingsStore,
    assets: Option<&ThemeAssetService>,
    default: Option<ThemeBackgroundAsset>,
) -> Result<(Option<ThemeBackgroundAsset>, bool), ThemeError> {
    let Some(value) = store.read_setting(THEME_BACKGROUND_ASSET_KEY)? else {
        return Ok((default, true));
    };
    if value.is_empty() {
        return Ok((None, false));
    }
    let asset = serde_json::from_str::<ThemeBackgroundAsset>(&value).map_err(|error| {
        corrupt(
            THEME_BACKGROUND_ASSET_KEY,
            format!("invalid background metadata: {error}"),
        )
    })?;
    asset.validate().map_err(|error| {
        corrupt(
            THEME_BACKGROUND_ASSET_KEY,
            format!("invalid background metadata: {error}"),
        )
    })?;
    if let Some(assets) = assets {
        if assets.read(&asset).is_err() {
            return Ok((None, true));
        }
    }
    Ok((Some(asset), false))
}

fn read_u8_setting(
    store: &dyn ThemeSettingsStore,
    key: &'static str,
    default: u8,
    minimum: u8,
    maximum: u8,
) -> Result<(u8, bool), ThemeError> {
    match store.read_setting(key)? {
        Some(value) => {
            let parsed = value
                .parse::<u8>()
                .map_err(|_| corrupt(key, format!("invalid numeric value {value}")))?;
            if !(minimum..=maximum).contains(&parsed) {
                return Err(corrupt(
                    key,
                    format!("value {parsed} outside {minimum}..={maximum}"),
                ));
            }
            Ok((parsed, false))
        }
        None => Ok((default, true)),
    }
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

    use image::{DynamicImage, ImageFormat, RgbaImage};
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
            accent: Some(AccentColor::parse("#f4e04d").unwrap()),
            animation_speed: Some(AnimationSpeed::Fast),
            reduce_transparency: Some(true),
            ..ThemePreferencesPatch::default()
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
    fn invalid_values_are_rejected_and_custom_accent_is_canonicalized() {
        assert!(matches!(
            ThemeMode::parse("sepia"),
            Err(ThemeValidationError::InvalidThemeMode(_))
        ));
        for invalid in ["ffffff", "#ffff", "#gggggg", "#12345678"] {
            assert!(matches!(
                AccentColor::parse(invalid),
                Err(ThemeValidationError::InvalidAccent(_))
            ));
        }
        assert!(matches!(
            AnimationSpeed::parse("instant"),
            Err(ThemeValidationError::InvalidAnimationSpeed(_))
        ));
        assert_eq!(AccentColor::parse("#C7427A").unwrap().as_str(), "#c7427a");
        assert_eq!(AccentColor::parse("#F4E04D").unwrap().as_str(), "#f4e04d");
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

    #[test]
    fn background_import_restart_and_removal_keep_settings_and_files_consistent() {
        let temp = tempdir().unwrap();
        let data_root = temp.path().join("app-data");
        let source = temp.path().join("wallpaper.jpg");
        DynamicImage::ImageRgba8(RgbaImage::from_raw(2, 2, [40, 60, 80, 255].repeat(4)).unwrap())
            .save_with_format(&source, ImageFormat::Jpeg)
            .unwrap();

        let storage = Arc::new(StorageService::initialize(&data_root).unwrap());
        let service = ThemeService::initialize(Arc::clone(&storage)).unwrap();
        let imported = service.import_background(0, &source).unwrap();
        let asset = imported.preferences.background.clone().unwrap();
        assert_eq!(asset.file_name, "wallpaper.jpg");
        assert!(service
            .read_background()
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n"));
        drop(service);
        drop(storage);

        let reopened_storage = Arc::new(StorageService::initialize(&data_root).unwrap());
        let reopened = ThemeService::initialize(reopened_storage).unwrap();
        assert_eq!(
            reopened.current().unwrap().preferences.background,
            Some(asset.clone())
        );
        let customized = reopened
            .update(
                imported.revision,
                ThemePreferencesPatch {
                    background_fit: Some(BackgroundFit::Contain),
                    background_dim: Some(64),
                    background_blur: Some(16),
                    panel_opacity: Some(70),
                    ..ThemePreferencesPatch::default()
                },
            )
            .unwrap();
        let removed = reopened.remove_background(customized.revision).unwrap();
        assert_eq!(removed.preferences.background, None);
        assert_eq!(removed.preferences.background_fit, BackgroundFit::Cover);
        assert_eq!(removed.preferences.background_dim, DEFAULT_BACKGROUND_DIM);
        assert_eq!(removed.preferences.background_blur, DEFAULT_BACKGROUND_BLUR);
        assert_eq!(removed.preferences.panel_opacity, DEFAULT_PANEL_OPACITY);
        assert!(matches!(
            reopened.read_background(),
            Err(ThemeError::Asset(ThemeAssetError::Missing))
        ));
        let asset_path = data_root
            .join("files/themes/backgrounds")
            .join(format!("{}.png", asset.id));
        assert!(!asset_path.exists());
    }

    #[test]
    fn material_ranges_are_validated_before_persistence() {
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        let service = ThemeService::initialize(storage).unwrap();
        for patch in [
            ThemePreferencesPatch {
                background_dim: Some(101),
                ..ThemePreferencesPatch::default()
            },
            ThemePreferencesPatch {
                background_blur: Some(25),
                ..ThemePreferencesPatch::default()
            },
        ] {
            assert!(matches!(
                service.update(0, patch),
                Err(ThemeError::Validation(_))
            ));
            assert_eq!(service.current().unwrap().revision, 0);
        }
    }
}
