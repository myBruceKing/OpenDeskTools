use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use thiserror::Error;

use super::storage::{StorageError, StorageService};

const HOTKEY_SNAPSHOT_KEY: &str = "hotkeys.snapshot.v1";
const HOTKEY_REVISION_KEY: &str = "hotkeys.revision";
const PERSISTENCE_VERSION: u32 = 1;

const UNAVAILABLE_DETAIL: &str = "功能尚未接入，当前不会注册或占用此快捷键。";
const SYSTEM_RESERVED_DETAIL: &str = "此组合由 Windows 使用；只有显式强制覆盖后才允许尝试接管。";
const BLOCKED_DETAIL: &str = "此系统安全组合不能被 OpenDeskTools 接管。";
const SEQUENCE_DETAIL: &str = "当前全局快捷键后端不支持连续按键。";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HotkeyActionId {
    #[serde(rename = "screenshot.capture")]
    ScreenshotCapture,
    #[serde(rename = "clipboard.pin_image")]
    ClipboardPinImage,
    #[serde(rename = "clipboard.qr_convert")]
    ClipboardQrConvert,
    #[serde(rename = "launcher.open")]
    LauncherOpen,
    #[serde(rename = "clipboard.open_panel")]
    ClipboardOpenPanel,
}

impl HotkeyActionId {
    pub const ALL: [Self; 5] = [
        Self::ScreenshotCapture,
        Self::ClipboardPinImage,
        Self::ClipboardQrConvert,
        Self::LauncherOpen,
        Self::ClipboardOpenPanel,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ScreenshotCapture => "screenshot.capture",
            Self::ClipboardPinImage => "clipboard.pin_image",
            Self::ClipboardQrConvert => "clipboard.qr_convert",
            Self::LauncherOpen => "launcher.open",
            Self::ClipboardOpenPanel => "clipboard.open_panel",
        }
    }

    pub fn parse(value: &str) -> Result<Self, HotkeyValidationError> {
        Self::ALL
            .into_iter()
            .find(|action| action.as_str() == value)
            .ok_or_else(|| HotkeyValidationError::UnknownAction(value.to_owned()))
    }

    fn default_binding(self) -> &'static str {
        match self {
            Self::ScreenshotCapture => "F1",
            Self::ClipboardPinImage => "F3",
            Self::ClipboardQrConvert => "F4",
            Self::LauncherOpen => "Alt+Space",
            Self::ClipboardOpenPanel => "Win+V",
        }
    }
}

impl fmt::Display for HotkeyActionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Modifiers(u8);

impl Modifiers {
    const CTRL: u8 = 1;
    const ALT: u8 = 2;
    const SHIFT: u8 = 4;
    const WIN: u8 = 8;

    fn contains(self, flag: u8) -> bool {
        self.0 & flag != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HotkeyChord {
    modifiers: Modifiers,
    key: String,
}

impl HotkeyChord {
    pub fn parse(value: &str) -> Result<Self, HotkeyValidationError> {
        let tokens = value.split('+').map(str::trim).collect::<Vec<_>>();
        if tokens.is_empty() || tokens.iter().any(|token| token.is_empty()) {
            return Err(HotkeyValidationError::InvalidBinding(value.to_owned()));
        }

        let mut modifier_bits = 0_u8;
        let mut key = None;
        for token in tokens {
            let upper = token.to_ascii_uppercase();
            let modifier = match upper.as_str() {
                "CTRL" | "CONTROL" => Some(Modifiers::CTRL),
                "ALT" | "OPTION" => Some(Modifiers::ALT),
                "SHIFT" => Some(Modifiers::SHIFT),
                "WIN" | "SUPER" | "META" | "CMD" | "COMMAND" => Some(Modifiers::WIN),
                _ => None,
            };
            if let Some(modifier) = modifier {
                if key.is_some() {
                    return Err(HotkeyValidationError::InvalidBinding(value.to_owned()));
                }
                modifier_bits |= modifier;
            } else if key.is_some() {
                return Err(HotkeyValidationError::InvalidBinding(value.to_owned()));
            } else {
                key = Some(normalize_key(token)?);
            }
        }

        Ok(Self {
            modifiers: Modifiers(modifier_bits),
            key: key.ok_or_else(|| HotkeyValidationError::InvalidBinding(value.to_owned()))?,
        })
    }

    pub fn normalized(&self) -> String {
        let mut parts = Vec::with_capacity(5);
        if self.modifiers.contains(Modifiers::CTRL) {
            parts.push("Ctrl");
        }
        if self.modifiers.contains(Modifiers::ALT) {
            parts.push("Alt");
        }
        if self.modifiers.contains(Modifiers::SHIFT) {
            parts.push("Shift");
        }
        if self.modifiers.contains(Modifiers::WIN) {
            parts.push("Win");
        }
        parts.push(self.key.as_str());
        parts.join("+")
    }

    fn plugin_binding(&self) -> String {
        self.normalized().replace("Win+", "Super+")
    }
}

fn normalize_key(value: &str) -> Result<String, HotkeyValidationError> {
    let upper = value.to_ascii_uppercase();
    if let Some(number) = upper.strip_prefix('F') {
        if number
            .parse::<u8>()
            .is_ok_and(|function_key| (1..=24).contains(&function_key))
        {
            return Ok(upper);
        }
    }
    if upper.len() == 1 && upper.chars().all(|key| key.is_ascii_alphanumeric()) {
        return Ok(upper);
    }
    if let Some(letter) = upper.strip_prefix("KEY") {
        if letter.len() == 1 && letter.chars().all(|key| key.is_ascii_alphabetic()) {
            return Ok(letter.to_owned());
        }
    }
    let normalized = match upper.as_str() {
        "SPACE" | "SPACEBAR" => "Space",
        "DELETE" | "DEL" => "Delete",
        "ESCAPE" | "ESC" => "Escape",
        "ENTER" | "RETURN" => "Enter",
        "TAB" => "Tab",
        "BACKSPACE" => "Backspace",
        "INSERT" => "Insert",
        "HOME" => "Home",
        "END" => "End",
        "PAGEUP" => "PageUp",
        "PAGEDOWN" => "PageDown",
        "ARROWUP" | "UP" => "ArrowUp",
        "ARROWDOWN" | "DOWN" => "ArrowDown",
        "ARROWLEFT" | "LEFT" => "ArrowLeft",
        "ARROWRIGHT" | "RIGHT" => "ArrowRight",
        "PRINTSCREEN" => "PrintScreen",
        _ => return Err(HotkeyValidationError::UnsupportedKey(value.to_owned())),
    };
    Ok(normalized.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyBinding {
    Chord(HotkeyChord),
    Sequence(Vec<HotkeyChord>),
}

impl HotkeyBinding {
    pub fn parse(value: &str) -> Result<Self, HotkeyValidationError> {
        if value.trim().is_empty() {
            return Err(HotkeyValidationError::InvalidBinding(value.to_owned()));
        }
        let normalized_arrows = value.replace("→", ",");
        let compact_pluses = normalized_arrows
            .split('+')
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("+");
        let pieces = if compact_pluses.contains(',') {
            compact_pluses.split(',').map(str::trim).collect::<Vec<_>>()
        } else {
            compact_pluses.split_whitespace().collect::<Vec<_>>()
        };
        if pieces.iter().any(|piece| piece.is_empty()) {
            return Err(HotkeyValidationError::InvalidBinding(value.to_owned()));
        }
        let chords = pieces
            .into_iter()
            .map(HotkeyChord::parse)
            .collect::<Result<Vec<_>, _>>()?;
        if chords.len() == 1 {
            Ok(Self::Chord(chords.into_iter().next().expect("one chord")))
        } else {
            Ok(Self::Sequence(chords))
        }
    }

    pub fn normalized(&self) -> String {
        match self {
            Self::Chord(chord) => chord.normalized(),
            Self::Sequence(chords) => chords
                .iter()
                .map(HotkeyChord::normalized)
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyBindingClassification {
    Ordinary,
    SystemReserved,
    Blocked,
    UnsupportedSequence,
}

pub fn classify_binding(value: &str) -> Result<HotkeyBindingClassification, HotkeyValidationError> {
    classify_parsed_binding(&HotkeyBinding::parse(value)?)
}

fn classify_parsed_binding(
    binding: &HotkeyBinding,
) -> Result<HotkeyBindingClassification, HotkeyValidationError> {
    let HotkeyBinding::Chord(chord) = binding else {
        return Ok(HotkeyBindingClassification::UnsupportedSequence);
    };

    let ctrl_alt_delete = chord.modifiers.contains(Modifiers::CTRL)
        && chord.modifiers.contains(Modifiers::ALT)
        && chord.key == "Delete";
    let win_l = chord.modifiers.contains(Modifiers::WIN) && chord.key == "L";
    if ctrl_alt_delete || win_l {
        return Ok(HotkeyBindingClassification::Blocked);
    }
    let shell_reserved = (chord.modifiers.contains(Modifiers::ALT)
        && matches!(chord.key.as_str(), "Space" | "Tab" | "Escape" | "F4"))
        || (chord.modifiers.contains(Modifiers::CTRL) && chord.key == "Escape")
        || chord.key == "PrintScreen";
    if chord.modifiers.contains(Modifiers::WIN) || shell_reserved {
        return Ok(HotkeyBindingClassification::SystemReserved);
    }
    Ok(HotkeyBindingClassification::Ordinary)
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum HotkeyValidationError {
    #[error("unknown hotkey action: {0}")]
    UnknownAction(String),
    #[error("invalid hotkey binding: {0}")]
    InvalidBinding(String),
    #[error("unsupported hotkey key: {0}")]
    UnsupportedKey(String),
    #[error("system-reserved shortcut requires explicit force override")]
    ForceRequired,
    #[error("force override only applies to system-reserved shortcuts")]
    ForceOverrideNotApplicable,
    #[error("blocked system shortcut cannot be configured")]
    Blocked,
    #[error("sequential shortcuts are not supported by the global shortcut backend")]
    UnsupportedSequence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyRuntimeState {
    Registered,
    Conflict,
    Disabled,
    Unavailable,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyRegistrationSnapshot {
    pub action_id: HotkeyActionId,
    pub binding: String,
    pub configured_enabled: bool,
    pub force_override_system: bool,
    pub action_available: bool,
    pub classification: HotkeyBindingClassification,
    pub runtime_state: HotkeyRuntimeState,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeySnapshot {
    pub actions: Vec<HotkeyRegistrationSnapshot>,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HotkeyPreference {
    action_id: HotkeyActionId,
    binding: HotkeyBinding,
    configured_enabled: bool,
    force_override_system: bool,
}

#[derive(Debug, Clone)]
struct HotkeyRegistration {
    preference: HotkeyPreference,
    action_available: bool,
    runtime_state: HotkeyRuntimeState,
    detail: Option<String>,
    token: Option<RegistrationToken>,
}

#[derive(Debug)]
struct HotkeyState {
    registrations: Vec<HotkeyRegistration>,
    revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationToken {
    binding: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrarFailureKind {
    Conflict,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrarFailure {
    pub kind: RegistrarFailureKind,
    pub detail: String,
}

pub trait HotkeyRegistrar: Send + Sync {
    fn register(
        &self,
        action_id: HotkeyActionId,
        binding: &HotkeyChord,
    ) -> Result<RegistrationToken, RegistrarFailure>;

    fn unregister(&self, token: &RegistrationToken) -> Result<(), RegistrarFailure>;
}

pub struct TauriHotkeyRegistrar<'a, R: Runtime> {
    app: &'a AppHandle<R>,
}

impl<'a, R: Runtime> TauriHotkeyRegistrar<'a, R> {
    pub fn new(app: &'a AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> HotkeyRegistrar for TauriHotkeyRegistrar<'_, R> {
    fn register(
        &self,
        _action_id: HotkeyActionId,
        binding: &HotkeyChord,
    ) -> Result<RegistrationToken, RegistrarFailure> {
        let plugin_binding = binding.plugin_binding();
        self.app
            .global_shortcut()
            .register(plugin_binding.as_str())
            .map_err(|error| RegistrarFailure {
                kind: RegistrarFailureKind::Conflict,
                detail: error.to_string(),
            })?;
        Ok(RegistrationToken {
            binding: plugin_binding,
        })
    }

    fn unregister(&self, token: &RegistrationToken) -> Result<(), RegistrarFailure> {
        self.app
            .global_shortcut()
            .unregister(token.binding.as_str())
            .map_err(|error| RegistrarFailure {
                kind: RegistrarFailureKind::Unavailable,
                detail: error.to_string(),
            })
    }
}

#[derive(Debug, Error)]
pub enum HotkeyError {
    #[error("hotkey storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("corrupt hotkey setting {key}: {reason}")]
    CorruptSettings { key: &'static str, reason: String },
    #[error("hotkey state lock is poisoned")]
    StateLockPoisoned,
    #[error("hotkey revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("hotkey revision overflow")]
    RevisionOverflow,
    #[error(transparent)]
    Validation(#[from] HotkeyValidationError),
}

pub(crate) trait HotkeySettingsStore: fmt::Debug + Send + Sync {
    fn read_setting(&self, key: &str) -> Result<Option<String>, StorageError>;
    fn write_settings(&self, settings: &[(&str, &str)]) -> Result<(), StorageError>;
}

impl HotkeySettingsStore for StorageService {
    fn read_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        StorageService::read_setting(self, key)
    }

    fn write_settings(&self, settings: &[(&str, &str)]) -> Result<(), StorageError> {
        StorageService::write_settings(self, settings)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedSnapshot {
    version: u32,
    registrations: Vec<PersistedRegistration>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedRegistration {
    action_id: HotkeyActionId,
    binding: String,
    configured_enabled: bool,
    force_override_system: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateHotkeyBinding {
    pub action_id: HotkeyActionId,
    pub expected_revision: u64,
    pub binding: String,
    pub force_override_system: bool,
}

#[derive(Debug)]
pub struct HotkeyManager {
    store: Arc<dyn HotkeySettingsStore>,
    state: Mutex<HotkeyState>,
}

impl HotkeyManager {
    pub fn initialize(storage: Arc<StorageService>) -> Result<Self, HotkeyError> {
        Self::initialize_with_store(storage)
    }

    pub(crate) fn initialize_with_store(
        store: Arc<dyn HotkeySettingsStore>,
    ) -> Result<Self, HotkeyError> {
        let (preferences, revision, missing) = load_preferences(store.as_ref())?;
        if missing {
            persist_preferences(store.as_ref(), &preferences, revision)?;
        }
        let registrations = preferences
            .into_iter()
            .map(|preference| build_registration(preference, false))
            .collect();
        Ok(Self {
            store,
            state: Mutex::new(HotkeyState {
                registrations,
                revision,
            }),
        })
    }

    pub fn snapshot(&self) -> Result<HotkeySnapshot, HotkeyError> {
        let state = self
            .state
            .lock()
            .map_err(|_| HotkeyError::StateLockPoisoned)?;
        Ok(snapshot_from_state(&state))
    }

    pub fn update_binding(
        &self,
        update: UpdateHotkeyBinding,
        registrar: &dyn HotkeyRegistrar,
    ) -> Result<HotkeySnapshot, HotkeyError> {
        let binding = HotkeyBinding::parse(&update.binding)?;
        let classification = classify_parsed_binding(&binding)?;
        validate_force_policy(classification, update.force_override_system)?;

        let mut state = self
            .state
            .lock()
            .map_err(|_| HotkeyError::StateLockPoisoned)?;
        if state.revision != update.expected_revision {
            return Err(HotkeyError::RevisionConflict {
                expected: update.expected_revision,
                actual: state.revision,
            });
        }
        let index = state
            .registrations
            .iter()
            .position(|entry| entry.preference.action_id == update.action_id)
            .ok_or_else(|| HotkeyValidationError::UnknownAction(update.action_id.to_string()))?;
        let revision = state
            .revision
            .checked_add(1)
            .ok_or(HotkeyError::RevisionOverflow)?;
        let mut next_preferences = state
            .registrations
            .iter()
            .map(|entry| entry.preference.clone())
            .collect::<Vec<_>>();
        next_preferences[index].binding = binding;
        next_preferences[index].force_override_system = update.force_override_system;

        // Persist first. With all current actions unavailable this is deliberately a
        // configuration-only operation and cannot touch the operating system.
        persist_preferences(self.store.as_ref(), &next_preferences, revision)?;
        for (entry, preference) in state.registrations.iter_mut().zip(next_preferences) {
            entry.preference = preference;
        }
        state.revision = revision;
        reconcile_locked(&mut state, registrar);
        Ok(snapshot_from_state(&state))
    }

    pub fn reconcile(
        &self,
        registrar: &dyn HotkeyRegistrar,
    ) -> Result<HotkeySnapshot, HotkeyError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| HotkeyError::StateLockPoisoned)?;
        reconcile_locked(&mut state, registrar);
        Ok(snapshot_from_state(&state))
    }

    pub fn registered_action_for_plugin_binding(
        &self,
        binding: &str,
    ) -> Option<(HotkeyActionId, u64)> {
        let binding = HotkeyBinding::parse(binding).ok()?;
        let normalized = binding.normalized();
        let state = self.state.lock().ok()?;
        state
            .registrations
            .iter()
            .find(|entry| {
                entry.runtime_state == HotkeyRuntimeState::Registered
                    && entry.preference.binding.normalized() == normalized
            })
            .map(|entry| (entry.preference.action_id, state.revision))
    }
}

fn validate_force_policy(
    classification: HotkeyBindingClassification,
    force_override_system: bool,
) -> Result<(), HotkeyValidationError> {
    match (classification, force_override_system) {
        (HotkeyBindingClassification::SystemReserved, false) => {
            Err(HotkeyValidationError::ForceRequired)
        }
        (HotkeyBindingClassification::SystemReserved, true)
        | (HotkeyBindingClassification::Ordinary, false) => Ok(()),
        (HotkeyBindingClassification::Ordinary, true) => {
            Err(HotkeyValidationError::ForceOverrideNotApplicable)
        }
        (HotkeyBindingClassification::Blocked, _) => Err(HotkeyValidationError::Blocked),
        (HotkeyBindingClassification::UnsupportedSequence, _) => {
            Err(HotkeyValidationError::UnsupportedSequence)
        }
    }
}

fn reconcile_locked(state: &mut HotkeyState, registrar: &dyn HotkeyRegistrar) {
    let mut active_bindings = HashMap::<String, usize>::new();
    for entry in &state.registrations {
        if should_register(entry) {
            *active_bindings
                .entry(entry.preference.binding.normalized())
                .or_default() += 1;
        }
    }

    for entry in &mut state.registrations {
        let classification = classify_parsed_binding(&entry.preference.binding)
            .expect("persisted bindings are validated before entering state");
        let duplicated = active_bindings
            .get(&entry.preference.binding.normalized())
            .copied()
            .unwrap_or_default()
            > 1;
        let desired_binding = match &entry.preference.binding {
            HotkeyBinding::Chord(chord)
                if entry.action_available
                    && entry.preference.configured_enabled
                    && is_registration_allowed(entry, classification)
                    && !duplicated =>
            {
                Some(chord)
            }
            _ => None,
        };

        if let Some(token) = entry.token.as_ref() {
            let token_is_current =
                desired_binding.is_some_and(|chord| token.binding == chord.plugin_binding());
            if token_is_current {
                entry.runtime_state = HotkeyRuntimeState::Registered;
                entry.detail = None;
                continue;
            }
            if registrar.unregister(token).is_err() {
                entry.runtime_state = HotkeyRuntimeState::Degraded;
                entry.detail =
                    Some("旧快捷键未能注销；已停止继续变更，请重启应用恢复。".to_owned());
                continue;
            }
            entry.token = None;
        }

        if !entry.action_available {
            entry.runtime_state = HotkeyRuntimeState::Unavailable;
            entry.detail = Some(detail_for_classification(classification).to_owned());
            continue;
        }
        if !entry.preference.configured_enabled {
            entry.runtime_state = HotkeyRuntimeState::Disabled;
            entry.detail = Some("快捷键未启用。".to_owned());
            continue;
        }
        if !is_registration_allowed(entry, classification) {
            entry.runtime_state = HotkeyRuntimeState::Unavailable;
            entry.detail = Some(detail_for_classification(classification).to_owned());
            continue;
        }
        if duplicated {
            entry.runtime_state = HotkeyRuntimeState::Conflict;
            entry.detail = Some("快捷键与另一个已启用操作重复。".to_owned());
            continue;
        }
        let HotkeyBinding::Chord(chord) = &entry.preference.binding else {
            unreachable!("registration policy excludes sequences")
        };
        match registrar.register(entry.preference.action_id, chord) {
            Ok(token) => {
                entry.token = Some(token);
                entry.runtime_state = HotkeyRuntimeState::Registered;
                entry.detail = None;
            }
            Err(error) => {
                entry.runtime_state = match error.kind {
                    RegistrarFailureKind::Conflict => HotkeyRuntimeState::Conflict,
                    RegistrarFailureKind::Unavailable => HotkeyRuntimeState::Degraded,
                };
                entry.detail = Some(match error.kind {
                    RegistrarFailureKind::Conflict => {
                        "系统拒绝注册，快捷键可能已被系统或其他程序占用。".to_owned()
                    }
                    RegistrarFailureKind::Unavailable => "快捷键注册后端暂时不可用。".to_owned(),
                });
            }
        }
    }
}

fn should_register(entry: &HotkeyRegistration) -> bool {
    let Ok(classification) = classify_parsed_binding(&entry.preference.binding) else {
        return false;
    };
    entry.action_available
        && entry.preference.configured_enabled
        && is_registration_allowed(entry, classification)
}

fn is_registration_allowed(
    entry: &HotkeyRegistration,
    classification: HotkeyBindingClassification,
) -> bool {
    match classification {
        HotkeyBindingClassification::Ordinary => true,
        HotkeyBindingClassification::SystemReserved => entry.preference.force_override_system,
        HotkeyBindingClassification::Blocked | HotkeyBindingClassification::UnsupportedSequence => {
            false
        }
    }
}

fn detail_for_classification(classification: HotkeyBindingClassification) -> &'static str {
    match classification {
        HotkeyBindingClassification::Ordinary => UNAVAILABLE_DETAIL,
        HotkeyBindingClassification::SystemReserved => SYSTEM_RESERVED_DETAIL,
        HotkeyBindingClassification::Blocked => BLOCKED_DETAIL,
        HotkeyBindingClassification::UnsupportedSequence => SEQUENCE_DETAIL,
    }
}

fn build_registration(preference: HotkeyPreference, action_available: bool) -> HotkeyRegistration {
    let classification = classify_parsed_binding(&preference.binding)
        .expect("default and persisted bindings are validated");
    HotkeyRegistration {
        preference,
        action_available,
        runtime_state: HotkeyRuntimeState::Unavailable,
        detail: Some(detail_for_classification(classification).to_owned()),
        token: None,
    }
}

fn snapshot_from_state(state: &HotkeyState) -> HotkeySnapshot {
    HotkeySnapshot {
        actions: state
            .registrations
            .iter()
            .map(|entry| HotkeyRegistrationSnapshot {
                action_id: entry.preference.action_id,
                binding: entry.preference.binding.normalized(),
                configured_enabled: entry.preference.configured_enabled,
                force_override_system: entry.preference.force_override_system,
                action_available: entry.action_available,
                classification: classify_parsed_binding(&entry.preference.binding)
                    .expect("state bindings are validated"),
                runtime_state: entry.runtime_state,
                detail: entry.detail.clone(),
            })
            .collect(),
        revision: state.revision,
    }
}

fn default_preferences() -> Vec<HotkeyPreference> {
    HotkeyActionId::ALL
        .into_iter()
        .map(|action_id| HotkeyPreference {
            action_id,
            binding: HotkeyBinding::parse(action_id.default_binding())
                .expect("built-in hotkeys must be valid"),
            configured_enabled: true,
            force_override_system: false,
        })
        .collect()
}

fn load_preferences(
    store: &dyn HotkeySettingsStore,
) -> Result<(Vec<HotkeyPreference>, u64, bool), HotkeyError> {
    let snapshot = store.read_setting(HOTKEY_SNAPSHOT_KEY)?;
    let revision = store.read_setting(HOTKEY_REVISION_KEY)?;
    match (snapshot, revision) {
        (None, None) => Ok((default_preferences(), 0, true)),
        (Some(_), None) => Err(corrupt(HOTKEY_REVISION_KEY, "missing revision")),
        (None, Some(_)) => Err(corrupt(HOTKEY_SNAPSHOT_KEY, "missing snapshot")),
        (Some(snapshot), Some(revision)) => {
            let persisted: PersistedSnapshot = serde_json::from_str(&snapshot)
                .map_err(|error| corrupt(HOTKEY_SNAPSHOT_KEY, error.to_string()))?;
            if persisted.version != PERSISTENCE_VERSION {
                return Err(corrupt(
                    HOTKEY_SNAPSHOT_KEY,
                    format!("unsupported snapshot version {}", persisted.version),
                ));
            }
            let revision = revision.parse::<u64>().map_err(|_| {
                corrupt(HOTKEY_REVISION_KEY, format!("invalid revision {revision}"))
            })?;
            let mut seen = HashSet::new();
            let mut preferences = Vec::with_capacity(HotkeyActionId::ALL.len());
            for registration in persisted.registrations {
                if !seen.insert(registration.action_id) {
                    return Err(corrupt(
                        HOTKEY_SNAPSHOT_KEY,
                        format!("duplicate action {}", registration.action_id),
                    ));
                }
                let binding = HotkeyBinding::parse(&registration.binding)
                    .map_err(|error| corrupt(HOTKEY_SNAPSHOT_KEY, error.to_string()))?;
                let classification = classify_parsed_binding(&binding)
                    .map_err(|error| corrupt(HOTKEY_SNAPSHOT_KEY, error.to_string()))?;
                let persisted_policy_is_valid = match classification {
                    HotkeyBindingClassification::Ordinary => !registration.force_override_system,
                    HotkeyBindingClassification::SystemReserved => true,
                    HotkeyBindingClassification::Blocked
                    | HotkeyBindingClassification::UnsupportedSequence => false,
                };
                if !persisted_policy_is_valid {
                    return Err(corrupt(
                        HOTKEY_SNAPSHOT_KEY,
                        format!("invalid persisted policy for {}", registration.action_id),
                    ));
                }
                preferences.push(HotkeyPreference {
                    action_id: registration.action_id,
                    binding,
                    configured_enabled: registration.configured_enabled,
                    force_override_system: registration.force_override_system,
                });
            }
            if seen.len() != HotkeyActionId::ALL.len()
                || HotkeyActionId::ALL
                    .iter()
                    .any(|action| !seen.contains(action))
            {
                return Err(corrupt(
                    HOTKEY_SNAPSHOT_KEY,
                    "snapshot must contain every known action exactly once",
                ));
            }
            preferences.sort_by_key(|preference| {
                HotkeyActionId::ALL
                    .iter()
                    .position(|action| *action == preference.action_id)
                    .expect("validated action")
            });
            Ok((preferences, revision, false))
        }
    }
}

fn persist_preferences(
    store: &dyn HotkeySettingsStore,
    preferences: &[HotkeyPreference],
    revision: u64,
) -> Result<(), HotkeyError> {
    let snapshot = PersistedSnapshot {
        version: PERSISTENCE_VERSION,
        registrations: preferences
            .iter()
            .map(|preference| PersistedRegistration {
                action_id: preference.action_id,
                binding: preference.binding.normalized(),
                configured_enabled: preference.configured_enabled,
                force_override_system: preference.force_override_system,
            })
            .collect(),
    };
    let snapshot = serde_json::to_string(&snapshot)
        .map_err(|error| corrupt(HOTKEY_SNAPSHOT_KEY, error.to_string()))?;
    let revision = revision.to_string();
    store.write_settings(&[
        (HOTKEY_SNAPSHOT_KEY, snapshot.as_str()),
        (HOTKEY_REVISION_KEY, revision.as_str()),
    ])?;
    Ok(())
}

fn corrupt(key: &'static str, reason: impl Into<String>) -> HotkeyError {
    HotkeyError::CorruptSettings {
        key,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    use tempfile::tempdir;

    use super::*;

    #[derive(Default)]
    struct FakeRegistrar {
        registrations: AtomicUsize,
    }

    #[derive(Debug, Default)]
    struct FailingStore {
        values: Mutex<HashMap<String, String>>,
        fail_writes: AtomicBool,
        write_count: AtomicUsize,
    }

    impl HotkeySettingsStore for FailingStore {
        fn read_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
            Ok(self.values.lock().unwrap().get(key).cloned())
        }

        fn write_settings(&self, settings: &[(&str, &str)]) -> Result<(), StorageError> {
            self.write_count.fetch_add(1, Ordering::SeqCst);
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

    impl HotkeyRegistrar for FakeRegistrar {
        fn register(
            &self,
            _action_id: HotkeyActionId,
            binding: &HotkeyChord,
        ) -> Result<RegistrationToken, RegistrarFailure> {
            self.registrations.fetch_add(1, Ordering::SeqCst);
            Ok(RegistrationToken {
                binding: binding.normalized(),
            })
        }

        fn unregister(&self, _token: &RegistrationToken) -> Result<(), RegistrarFailure> {
            Ok(())
        }
    }

    #[test]
    fn classification_uses_parsed_modifiers_and_safety_precedence() {
        assert_eq!(
            classify_binding("F1").unwrap(),
            HotkeyBindingClassification::Ordinary
        );
        assert_eq!(
            classify_binding("Alt + Space").unwrap(),
            HotkeyBindingClassification::SystemReserved
        );
        assert_eq!(
            classify_binding("Win+R").unwrap(),
            HotkeyBindingClassification::SystemReserved
        );
        assert_eq!(
            classify_binding("Win+V").unwrap(),
            HotkeyBindingClassification::SystemReserved
        );
        assert_eq!(
            classify_binding("Win+K").unwrap(),
            HotkeyBindingClassification::SystemReserved
        );
        assert_eq!(
            classify_binding("Ctrl+Alt+Delete").unwrap(),
            HotkeyBindingClassification::Blocked
        );
        assert_eq!(
            classify_binding("Win+L").unwrap(),
            HotkeyBindingClassification::Blocked
        );
        assert_eq!(
            classify_binding("F1, Ctrl+V").unwrap(),
            HotkeyBindingClassification::UnsupportedSequence
        );
        assert_eq!(
            classify_binding("F1 F2").unwrap(),
            HotkeyBindingClassification::UnsupportedSequence
        );

        for binding in [
            "Alt+Tab",
            "Ctrl+Alt+Tab",
            "Alt+Esc",
            "Ctrl+Esc",
            "Ctrl+Shift+Esc",
            "PrintScreen",
            "Alt+F4",
        ] {
            assert_eq!(
                classify_binding(binding).unwrap(),
                HotkeyBindingClassification::SystemReserved,
                "{binding} should be treated as a Windows shell shortcut"
            );
        }
    }

    #[test]
    fn chord_parser_normalizes_aliases_and_modifier_order() {
        assert!(matches!(
            HotkeyBinding::parse(""),
            Err(HotkeyValidationError::InvalidBinding(_))
        ));
        assert_eq!(
            HotkeyBinding::parse("shift + control + keyv")
                .unwrap()
                .normalized(),
            "Ctrl+Shift+V"
        );
    }

    #[test]
    fn defaults_are_persisted_and_all_unavailable_actions_skip_registrar() {
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        let manager = HotkeyManager::initialize(Arc::clone(&storage)).unwrap();
        let registrar = FakeRegistrar::default();

        let snapshot = manager.reconcile(&registrar).unwrap();

        assert_eq!(snapshot.revision, 0);
        assert_eq!(snapshot.actions.len(), 5);
        assert!(snapshot.actions.iter().all(|entry| entry.configured_enabled
            && !entry.action_available
            && !entry.force_override_system));
        assert_eq!(registrar.registrations.load(Ordering::SeqCst), 0);
        assert!(storage.read_setting(HOTKEY_SNAPSHOT_KEY).unwrap().is_some());
        assert_eq!(
            storage
                .read_setting(HOTKEY_REVISION_KEY)
                .unwrap()
                .as_deref(),
            Some("0")
        );
    }

    #[test]
    fn unavailable_action_update_persists_without_registering() {
        let temp = tempdir().unwrap();
        let data_root = temp.path().join("data");
        let storage = Arc::new(StorageService::initialize(&data_root).unwrap());
        let manager = HotkeyManager::initialize(Arc::clone(&storage)).unwrap();
        let registrar = FakeRegistrar::default();

        let updated = manager
            .update_binding(
                UpdateHotkeyBinding {
                    action_id: HotkeyActionId::ScreenshotCapture,
                    expected_revision: 0,
                    binding: "Ctrl+Shift+S".to_owned(),
                    force_override_system: false,
                },
                &registrar,
            )
            .unwrap();

        assert_eq!(updated.revision, 1);
        assert_eq!(updated.actions[0].binding, "Ctrl+Shift+S");
        assert_eq!(registrar.registrations.load(Ordering::SeqCst), 0);
        drop(manager);
        drop(storage);
        let reopened = Arc::new(StorageService::initialize(&data_root).unwrap());
        let restored = HotkeyManager::initialize(reopened)
            .unwrap()
            .snapshot()
            .unwrap();
        assert_eq!(restored, updated);
    }

    #[test]
    fn unavailable_system_binding_can_store_explicit_force_without_registering() {
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        let manager = HotkeyManager::initialize(storage).unwrap();
        let registrar = FakeRegistrar::default();

        let updated = manager
            .update_binding(
                UpdateHotkeyBinding {
                    action_id: HotkeyActionId::ClipboardOpenPanel,
                    expected_revision: 0,
                    binding: "Win+V".to_owned(),
                    force_override_system: true,
                },
                &registrar,
            )
            .unwrap();

        assert_eq!(updated.revision, 1);
        assert!(updated.actions[4].force_override_system);
        assert_eq!(
            updated.actions[4].classification,
            HotkeyBindingClassification::SystemReserved
        );
        assert_eq!(
            updated.actions[4].runtime_state,
            HotkeyRuntimeState::Unavailable
        );
        assert_eq!(registrar.registrations.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn snapshot_serializes_stable_action_ids_and_separate_runtime_fields() {
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        let manager = HotkeyManager::initialize(storage).unwrap();

        let json = serde_json::to_value(manager.snapshot().unwrap()).unwrap();

        assert_eq!(json["revision"], 0);
        assert_eq!(json["actions"][0]["actionId"], "screenshot.capture");
        assert_eq!(json["actions"][0]["configuredEnabled"], true);
        assert_eq!(json["actions"][0]["actionAvailable"], false);
        assert_eq!(json["actions"][0]["runtimeState"], "unavailable");
        assert_eq!(json["actions"][4]["actionId"], "clipboard.open_panel");
    }

    #[test]
    fn failed_persistence_keeps_snapshot_revision_and_store_unchanged() {
        let store = Arc::new(FailingStore::default());
        let manager = HotkeyManager::initialize_with_store(store.clone()).unwrap();
        let registrar = FakeRegistrar::default();
        let snapshot_before = manager.snapshot().unwrap();
        let values_before = store.values.lock().unwrap().clone();
        let writes_before = store.write_count.load(Ordering::SeqCst);
        store.fail_writes.store(true, Ordering::SeqCst);

        let error = manager
            .update_binding(
                UpdateHotkeyBinding {
                    action_id: HotkeyActionId::ScreenshotCapture,
                    expected_revision: 0,
                    binding: "F2".to_owned(),
                    force_override_system: false,
                },
                &registrar,
            )
            .unwrap_err();

        assert!(matches!(error, HotkeyError::Storage(_)));
        assert_eq!(manager.snapshot().unwrap(), snapshot_before);
        assert_eq!(*store.values.lock().unwrap(), values_before);
        assert_eq!(store.write_count.load(Ordering::SeqCst), writes_before + 1);
        assert_eq!(registrar.registrations.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn rejected_force_policy_and_stale_revision_have_no_side_effects() {
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        let manager = HotkeyManager::initialize(Arc::clone(&storage)).unwrap();
        let registrar = FakeRegistrar::default();
        let before = storage.read_setting(HOTKEY_SNAPSHOT_KEY).unwrap();
        let revision_before = storage.read_setting(HOTKEY_REVISION_KEY).unwrap();

        for update in [
            UpdateHotkeyBinding {
                action_id: HotkeyActionId::ScreenshotCapture,
                expected_revision: 0,
                binding: "Win+R".to_owned(),
                force_override_system: false,
            },
            UpdateHotkeyBinding {
                action_id: HotkeyActionId::ScreenshotCapture,
                expected_revision: 0,
                binding: "F2".to_owned(),
                force_override_system: true,
            },
            UpdateHotkeyBinding {
                action_id: HotkeyActionId::ScreenshotCapture,
                expected_revision: 0,
                binding: "Ctrl+Alt+Delete".to_owned(),
                force_override_system: true,
            },
            UpdateHotkeyBinding {
                action_id: HotkeyActionId::ScreenshotCapture,
                expected_revision: 0,
                binding: "F1, F2".to_owned(),
                force_override_system: false,
            },
        ] {
            assert!(manager.update_binding(update, &registrar).is_err());
        }
        assert!(matches!(
            manager.update_binding(
                UpdateHotkeyBinding {
                    action_id: HotkeyActionId::ScreenshotCapture,
                    expected_revision: 99,
                    binding: "F2".to_owned(),
                    force_override_system: false,
                },
                &registrar
            ),
            Err(HotkeyError::RevisionConflict { .. })
        ));
        assert_eq!(manager.snapshot().unwrap().revision, 0);
        assert_eq!(storage.read_setting(HOTKEY_SNAPSHOT_KEY).unwrap(), before);
        assert_eq!(
            storage.read_setting(HOTKEY_REVISION_KEY).unwrap(),
            revision_before
        );
        assert_eq!(registrar.registrations.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn corrupt_snapshot_returns_explicit_error_without_overwrite() {
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        storage
            .write_settings(&[
                (HOTKEY_SNAPSHOT_KEY, "{not-json"),
                (HOTKEY_REVISION_KEY, "4"),
            ])
            .unwrap();

        let error = HotkeyManager::initialize(Arc::clone(&storage)).unwrap_err();

        assert!(matches!(error, HotkeyError::CorruptSettings { .. }));
        assert_eq!(
            storage
                .read_setting(HOTKEY_SNAPSHOT_KEY)
                .unwrap()
                .as_deref(),
            Some("{not-json")
        );
    }

    #[test]
    fn invalid_persisted_force_policy_is_rejected_without_overwrite() {
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        let manager = HotkeyManager::initialize(Arc::clone(&storage)).unwrap();
        drop(manager);

        let original = storage.read_setting(HOTKEY_SNAPSHOT_KEY).unwrap().unwrap();
        let mut json: serde_json::Value = serde_json::from_str(&original).unwrap();
        json["registrations"][0]["forceOverrideSystem"] = serde_json::Value::Bool(true);
        let invalid = serde_json::to_string(&json).unwrap();
        storage
            .write_settings(&[(HOTKEY_SNAPSHOT_KEY, invalid.as_str())])
            .unwrap();

        let error = HotkeyManager::initialize(Arc::clone(&storage)).unwrap_err();

        assert!(matches!(error, HotkeyError::CorruptSettings { .. }));
        assert_eq!(
            storage
                .read_setting(HOTKEY_SNAPSHOT_KEY)
                .unwrap()
                .as_deref(),
            Some(invalid.as_str())
        );
    }
}
