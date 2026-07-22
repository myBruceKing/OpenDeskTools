use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::source_icon::{SourceIconError, SourceIconService};
use super::storage::{StorageError, StorageService};

const SETTINGS_KEY: &str = "quick_launch.pinned.v1";
const MAX_DISCOVERED_APPS: usize = 300;
const MAX_SCAN_DEPTH: usize = 6;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickLaunchApp {
    pub id: String,
    pub name: String,
    pub path: String,
    pub arguments: String,
    pub working_directory: Option<String>,
    pub icon_path: String,
    pub icon_index: i32,
    pub source: String,
    pub visible: bool,
    pub available: bool,
    pub icon_available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickLaunchSnapshot {
    pub pinned_apps: Vec<QuickLaunchApp>,
    pub discovered_apps: Vec<QuickLaunchApp>,
    pub tool_menu: ToolMenuPreferences,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolMenuLayout { Wheel, Dock, Vertical }

impl Default for ToolMenuLayout {
    fn default() -> Self { Self::Wheel }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolMenuPreferences {
    #[serde(default)]
    pub layout: ToolMenuLayout,
    #[serde(default)]
    pub keep_open_on_key_release: bool,
}

impl Default for ToolMenuPreferences {
    fn default() -> Self { Self { layout: ToolMenuLayout::Wheel, keep_open_on_key_release: false } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedQuickLaunchApp {
    name: String,
    path: String,
    #[serde(default)]
    arguments: String,
    #[serde(default)]
    working_directory: Option<String>,
    #[serde(default)]
    icon_path: String,
    #[serde(default)]
    icon_index: i32,
    source: String,
    visible: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SavedQuickLaunchState {
    #[serde(default)]
    pinned_apps: Vec<SavedQuickLaunchApp>,
    #[serde(default)]
    tool_menu: ToolMenuPreferences,
}

#[derive(Debug, Error)]
pub enum QuickLaunchError {
    #[error("quick launch storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("quick launch icon failed: {0}")]
    Icon(#[from] SourceIconError),
    #[error("the selected application path is invalid")]
    InvalidPath,
    #[error("the selected application is already pinned")]
    AlreadyPinned,
    #[error("the selected application is not pinned")]
    NotPinned,
    #[error("the selected application target is unavailable")]
    Unavailable,
    #[error("failed to launch the selected application")]
    LaunchFailed,
    #[error("quick launch discovery state is unavailable")]
    DiscoveryState,
}

#[derive(Debug)]
pub struct QuickLaunchService {
    storage: Arc<StorageService>,
    icons: SourceIconService,
    discovered: Mutex<Vec<SavedQuickLaunchApp>>,
}

impl QuickLaunchService {
    pub fn initialize(storage: Arc<StorageService>) -> Result<Self, QuickLaunchError> {
        Ok(Self {
            icons: SourceIconService::initialize(Arc::clone(&storage))?,
            storage,
            discovered: Mutex::new(Vec::new()),
        })
    }

    pub fn snapshot(&self) -> Result<QuickLaunchSnapshot, QuickLaunchError> {
        let state = self.read_state()?;
        let discovered = self
            .discovered
            .lock()
            .map_err(|_| QuickLaunchError::DiscoveryState)?
            .clone();
        let pinned_paths = state
            .pinned_apps
            .iter()
            .map(|app| normalized_path(&app.path))
            .collect::<HashSet<_>>();

        let pinned_apps = state
            .pinned_apps
            .into_iter()
            .map(|app| self.to_view_model(app))
            .collect();
        let discovered_apps = discovered
            .into_iter()
            .filter(|app| !pinned_paths.contains(&normalized_path(&app.path)))
            .map(|app| self.to_view_model(app))
            .collect();

        Ok(QuickLaunchSnapshot {
            pinned_apps,
            discovered_apps,
            tool_menu: state.tool_menu,
        })
    }

    pub fn tool_menu_preferences(&self) -> Result<ToolMenuPreferences, QuickLaunchError> {
        Ok(self.read_state()?.tool_menu)
    }

    pub fn update_tool_menu_preferences(&self, preferences: ToolMenuPreferences) -> Result<QuickLaunchSnapshot, QuickLaunchError> {
        let mut state = self.read_state()?;
        state.tool_menu = preferences;
        self.write_state(&state)?;
        self.snapshot()
    }

    /// Performs the expensive desktop / Start Menu / App Paths enumeration on
    /// the caller's worker thread, then atomically publishes the new cache.
    /// Normal snapshots deliberately never rescan the filesystem.
    pub fn rescan(&self) -> Result<QuickLaunchSnapshot, QuickLaunchError> {
        let discovered = self.discover_apps();
        *self
            .discovered
            .lock()
            .map_err(|_| QuickLaunchError::DiscoveryState)? = discovered;
        self.snapshot()
    }

    pub fn pin(&self, path: String, source: Option<String>) -> Result<QuickLaunchSnapshot, QuickLaunchError> {
        let resolved = match self
            .discovered
            .lock()
            .map_err(|_| QuickLaunchError::DiscoveryState)?
            .clone()
            .into_iter()
            .find(|app| normalized_path(&app.path) == normalized_path(&path))
        {
            Some(app) => app,
            None => saved_app(
                resolve_launch_target(&path).ok_or(QuickLaunchError::InvalidPath)?,
                source.as_deref().unwrap_or("手动添加"),
            ),
        };
        let mut state = self.read_state()?;
        if state
            .pinned_apps
            .iter()
            .any(|app| normalized_path(&app.path) == normalized_path(&resolved.path))
        {
            return Err(QuickLaunchError::AlreadyPinned);
        }
        state.pinned_apps.push(SavedQuickLaunchApp {
            name: resolved.name,
            path: resolved.path,
            arguments: resolved.arguments,
            working_directory: resolved.working_directory,
            icon_path: resolved.icon_path,
            icon_index: resolved.icon_index,
            source: source.unwrap_or_else(|| "手动添加".to_owned()),
            visible: true,
        });
        self.write_state(&state)?;
        self.snapshot()
    }

    pub fn set_visible(&self, path: &str, visible: bool) -> Result<QuickLaunchSnapshot, QuickLaunchError> {
        let mut state = self.read_state()?;
        let Some(app) = state
            .pinned_apps
            .iter_mut()
            .find(|app| normalized_path(&app.path) == normalized_path(path))
        else {
            return Err(QuickLaunchError::NotPinned);
        };
        app.visible = visible;
        self.write_state(&state)?;
        self.snapshot()
    }

    pub fn unpin(&self, path: &str) -> Result<QuickLaunchSnapshot, QuickLaunchError> {
        let mut state = self.read_state()?;
        let before = state.pinned_apps.len();
        state
            .pinned_apps
            .retain(|app| normalized_path(&app.path) != normalized_path(path));
        if state.pinned_apps.len() == before {
            return Err(QuickLaunchError::NotPinned);
        }
        self.write_state(&state)?;
        self.snapshot()
    }

    pub fn reorder(&self, active_path: &str, over_path: &str) -> Result<QuickLaunchSnapshot, QuickLaunchError> {
        let mut state = self.read_state()?;
        let active = state
            .pinned_apps
            .iter()
            .position(|app| normalized_path(&app.path) == normalized_path(active_path))
            .ok_or(QuickLaunchError::NotPinned)?;
        let over = state
            .pinned_apps
            .iter()
            .position(|app| normalized_path(&app.path) == normalized_path(over_path))
            .ok_or(QuickLaunchError::NotPinned)?;
        if active != over {
            let app = state.pinned_apps.remove(active);
            state.pinned_apps.insert(over, app);
            self.write_state(&state)?;
        }
        self.snapshot()
    }

    pub fn launch(&self, path: &str) -> Result<(), QuickLaunchError> {
        let state = self.read_state()?;
        let app = state
            .pinned_apps
            .into_iter()
            .find(|app| normalized_path(&app.path) == normalized_path(path))
            .ok_or(QuickLaunchError::NotPinned)?;
        if !Path::new(&app.path).is_file() {
            return Err(QuickLaunchError::Unavailable);
        }
        launch_path(&app.path, &app.arguments, app.working_directory.as_deref())
    }

    pub fn icon_bytes(&self, path: &str) -> Result<Vec<u8>, QuickLaunchError> {
        let snapshot = self.snapshot()?;
        let app = snapshot
            .pinned_apps
            .into_iter()
            .chain(snapshot.discovered_apps)
            .find(|app| normalized_path(&app.path) == normalized_path(path))
            .ok_or(QuickLaunchError::Unavailable)?;
        let reference = self
            .icons
            .cache_icon(Path::new(&app.icon_path), app.icon_index)?
            .ok_or(QuickLaunchError::Unavailable)?;
        self.icons.read(&reference).map_err(Into::into)
    }

    fn to_view_model(&self, app: SavedQuickLaunchApp) -> QuickLaunchApp {
        let available = Path::new(&app.path).is_file();
        let icon_path = non_empty(app.icon_path.clone()).unwrap_or_else(|| app.path.clone());
        // Snapshot generation must never synchronously invoke the Shell icon
        // extractor for every discovered program. The UI asks for icons only
        // when a fixed item is actually displayed, keeping scans responsive.
        let icon_available = available;
        QuickLaunchApp {
            id: stable_id(&app.path),
            name: app.name,
            path: app.path,
            arguments: app.arguments,
            working_directory: app.working_directory,
            icon_path,
            icon_index: app.icon_index,
            source: app.source,
            visible: app.visible,
            available,
            icon_available,
        }
    }

    fn read_state(&self) -> Result<SavedQuickLaunchState, QuickLaunchError> {
        let Some(raw) = self.storage.read_setting(SETTINGS_KEY)? else {
            return Ok(SavedQuickLaunchState::default());
        };
        Ok(serde_json::from_str(&raw).unwrap_or_default())
    }

    fn write_state(&self, state: &SavedQuickLaunchState) -> Result<(), QuickLaunchError> {
        let payload = serde_json::to_string(state).map_err(|_| QuickLaunchError::InvalidPath)?;
        self.storage.write_settings(&[(SETTINGS_KEY, &payload)])?;
        Ok(())
    }

    fn discover_apps(&self) -> Vec<SavedQuickLaunchApp> {
        let mut candidates = HashMap::new();
        for (root, source) in discovery_roots() {
            collect_launchable_files(&root, &source, &mut candidates);
            if candidates.len() >= MAX_DISCOVERED_APPS {
                break;
            }
        }
        for path in registry_app_paths() {
            if candidates.len() >= MAX_DISCOVERED_APPS { break; }
            let path = path.to_string_lossy().into_owned();
            if let Some(app) = resolve_launch_target(&path) {
                candidates.entry(normalized_path(&app.path)).or_insert_with(|| saved_app(app, "注册表 App Paths"));
            }
        }
        let mut apps = candidates.into_values().collect::<Vec<_>>();
        apps.sort_by_key(|app| app.name.to_lowercase());
        apps.truncate(MAX_DISCOVERED_APPS);
        apps
    }
}

fn normalized_path(path: &str) -> String {
    path.trim().to_ascii_lowercase()
}

fn stable_id(path: &str) -> String {
    format!("app-{:x}", Sha256::digest(normalized_path(path).as_bytes()))
}

fn display_name(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(OsStr::to_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(path)
        .to_owned()
}

fn is_launchable(path: &Path) -> bool {
    matches!(
        path.extension().and_then(OsStr::to_str).map(|value| value.to_ascii_lowercase()),
        Some(extension) if extension == "exe" || extension == "lnk"
    )
}

fn collect_launchable_files(root: &Path, source: &str, candidates: &mut HashMap<String, SavedQuickLaunchApp>) {
    let mut directories = vec![(root.to_path_buf(), 0_usize)];
    while let Some((directory, depth)) = directories.pop() {
        let Ok(entries) = std::fs::read_dir(directory) else { continue; };
        for entry in entries.flatten() {
            if candidates.len() >= MAX_DISCOVERED_APPS { return; }
            let path = entry.path();
            if path.is_dir() && depth < MAX_SCAN_DEPTH {
                directories.push((path, depth + 1));
            } else if path.is_file() && is_launchable(&path) {
                if let Some(app) = resolve_launch_target(&path.to_string_lossy()) {
                    candidates.entry(normalized_path(&app.path)).or_insert_with(|| saved_app(app, source));
                }
            }
        }
    }
}

fn discovery_roots() -> Vec<(PathBuf, String)> {
    let mut roots = Vec::new();
    if let Ok(profile) = std::env::var("USERPROFILE") {
        roots.push((PathBuf::from(profile).join("Desktop"), "桌面".to_owned()));
    }
    if let Ok(app_data) = std::env::var("APPDATA") {
        roots.push((PathBuf::from(app_data).join("Microsoft/Windows/Start Menu/Programs"), "开始菜单".to_owned()));
    }
    if let Ok(program_data) = std::env::var("PROGRAMDATA") {
        roots.push((PathBuf::from(program_data).join("Microsoft/Windows/Start Menu/Programs"), "公共开始菜单".to_owned()));
    }
    roots
}

#[cfg(windows)]
fn registry_app_paths() -> Vec<PathBuf> {
    use windows_sys::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
        HKEY_LOCAL_MACHINE, KEY_READ, REG_EXPAND_SZ, REG_SZ,
    };
    const APP_PATHS: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths";
    let mut paths = Vec::new();
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let root_name = APP_PATHS.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let mut root_key: HKEY = std::ptr::null_mut();
        if unsafe { RegOpenKeyExW(root, root_name.as_ptr(), 0, KEY_READ, &mut root_key) } != ERROR_SUCCESS { continue; }
        let mut index = 0;
        loop {
            let mut name = vec![0_u16; 260];
            let mut length = name.len() as u32;
            let status = unsafe { RegEnumKeyExW(root_key, index, name.as_mut_ptr(), &mut length, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
            if status == ERROR_NO_MORE_ITEMS { break; }
            index += 1;
            if status != ERROR_SUCCESS { continue; }
            name.truncate(length as usize);
            let mut child: HKEY = std::ptr::null_mut();
            if unsafe { RegOpenKeyExW(root_key, name.as_ptr(), 0, KEY_READ, &mut child) } != ERROR_SUCCESS { continue; }
            let mut value_type = 0_u32;
            let mut byte_length = 0_u32;
            let first = unsafe { RegQueryValueExW(child, std::ptr::null(), std::ptr::null(), &mut value_type, std::ptr::null_mut(), &mut byte_length) };
            if first == ERROR_SUCCESS && (value_type == REG_SZ || value_type == REG_EXPAND_SZ) && byte_length >= 2 {
                let mut value = vec![0_u16; (byte_length as usize).div_ceil(2)];
                let mut size = (value.len() * std::mem::size_of::<u16>()) as u32;
                if unsafe { RegQueryValueExW(child, std::ptr::null(), std::ptr::null(), &mut value_type, value.as_mut_ptr() as *mut u8, &mut size) } == ERROR_SUCCESS {
                    let raw = String::from_utf16_lossy(&value).trim_end_matches('\0').trim().to_owned();
                    let expanded = if value_type == REG_EXPAND_SZ { expand_environment_variables(&raw) } else { raw };
                    if let Some(path) = executable_from_app_path(&expanded) { paths.push(path); }
                }
            }
            unsafe { RegCloseKey(child); }
        }
        unsafe { RegCloseKey(root_key); }
    }
    paths
}

#[cfg(windows)]
fn expand_environment_variables(value: &str) -> String {
    let mut result = value.to_owned();
    for (name, replacement) in std::env::vars() {
        result = result.replace(&format!("%{name}%"), &replacement);
    }
    result
}

#[cfg(windows)]
fn executable_from_app_path(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    let candidate = if let Some(rest) = value.strip_prefix('"') { rest.split('"').next()? } else { value };
    let candidate = PathBuf::from(candidate);
    (candidate.is_file() && is_launchable(&candidate)).then_some(candidate)
}

#[cfg(not(windows))]
fn registry_app_paths() -> Vec<PathBuf> { Vec::new() }

#[derive(Debug)]
struct ResolvedLaunchTarget {
    name: String,
    path: String,
    arguments: String,
    working_directory: Option<String>,
    icon_path: String,
    icon_index: i32,
}

fn saved_app(app: ResolvedLaunchTarget, source: &str) -> SavedQuickLaunchApp {
    SavedQuickLaunchApp {
        name: app.name,
        path: app.path,
        arguments: app.arguments,
        working_directory: app.working_directory,
        icon_path: app.icon_path,
        icon_index: app.icon_index,
        source: source.to_owned(),
        visible: true,
    }
}

fn resolve_launch_target(path: &str) -> Option<ResolvedLaunchTarget> {
    let path = Path::new(path);
    if !path.is_absolute() || !path.is_file() { return None; }
    match path.extension().and_then(OsStr::to_str).map(|value| value.to_ascii_lowercase()).as_deref() {
        Some("exe") => {
            let path = path.to_string_lossy().into_owned();
            Some(ResolvedLaunchTarget { name: display_name(&path), icon_path: path.clone(), path, arguments: String::new(), working_directory: None, icon_index: 0 })
        }
        Some("lnk") => resolve_windows_shortcut(path),
        _ => None,
    }
}

#[cfg(windows)]
fn resolve_windows_shortcut(shortcut: &Path) -> Option<ResolvedLaunchTarget> {
    use windows::core::{GUID, HSTRING, Interface};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ};
    use windows::Win32::UI::Shell::{IShellLinkW, SLGP_RAWPATH, SLR_NO_UI};
    const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() };
    let result = (|| unsafe {
        let link: IShellLinkW = CoCreateInstance(&CLSID_SHELL_LINK, None, CLSCTX_INPROC_SERVER).ok()?;
        let persist: IPersistFile = link.cast().ok()?;
        persist.Load(&HSTRING::from(shortcut.to_string_lossy().as_ref()), STGM_READ).ok()?;
        let _ = link.Resolve(HWND(std::ptr::null_mut()), SLR_NO_UI.0 as u32);
        let mut target = vec![0_u16; 32_768];
        link.GetPath(&mut target, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32).ok()?;
        let path = nul_terminated_string(&target);
        if !Path::new(&path).is_file() || !path.to_ascii_lowercase().ends_with(".exe") { return None; }
        let mut arguments = vec![0_u16; 32_768];
        link.GetArguments(&mut arguments).ok()?;
        let mut working_directory = vec![0_u16; 32_768];
        link.GetWorkingDirectory(&mut working_directory).ok()?;
        let mut icon_path = vec![0_u16; 32_768];
        let mut icon_index = 0_i32;
        link.GetIconLocation(&mut icon_path, &mut icon_index).ok()?;
        let icon_path = nul_terminated_string(&icon_path);
        Some(ResolvedLaunchTarget {
            name: display_name(&path), path: path.clone(), arguments: nul_terminated_string(&arguments),
            working_directory: non_empty(nul_terminated_string(&working_directory)),
            icon_path: non_empty(icon_path).unwrap_or(path), icon_index,
        })
    })();
    if initialized { unsafe { CoUninitialize(); } }
    result
}

#[cfg(not(windows))]
fn resolve_windows_shortcut(_shortcut: &Path) -> Option<ResolvedLaunchTarget> { None }

fn nul_terminated_string(buffer: &[u16]) -> String {
    let length = buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..length]).trim().to_owned()
}

fn non_empty(value: String) -> Option<String> { (!value.is_empty()).then_some(value) }

#[cfg(windows)]
fn launch_path(path: &str, arguments: &str, working_directory: Option<&str>) -> Result<(), QuickLaunchError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let wide = OsStr::new(path).encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let arguments = (!arguments.is_empty()).then(|| OsStr::new(arguments).encode_wide().chain(Some(0)).collect::<Vec<_>>());
    let directory = working_directory.map(|value| OsStr::new(value).encode_wide().chain(Some(0)).collect::<Vec<_>>());
    let result = unsafe { ShellExecuteW(std::ptr::null_mut(), std::ptr::null(), wide.as_ptr(), arguments.as_ref().map_or(std::ptr::null(), |value| value.as_ptr()), directory.as_ref().map_or(std::ptr::null(), |value| value.as_ptr()), 1) } as isize;
    if result <= 32 { Err(QuickLaunchError::LaunchFailed) } else { Ok(()) }
}

#[cfg(not(windows))]
fn launch_path(_path: &str, _arguments: &str, _working_directory: Option<&str>) -> Result<(), QuickLaunchError> { Err(QuickLaunchError::LaunchFailed) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_launchable_extensions_are_stable() {
        assert_eq!(stable_id(r"C:\\Apps\\Demo.exe"), stable_id(r"c:\\apps\\demo.EXE"));
        assert!(is_launchable(Path::new("demo.lnk")));
        assert!(!is_launchable(Path::new("note.txt")));
    }

    #[test]
    fn fixed_apps_persist_visibility_and_order_without_touching_system_launch_state() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("First.exe");
        let second = temp.path().join("Second.exe");
        std::fs::write(&first, []).unwrap();
        std::fs::write(&second, []).unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        let service = QuickLaunchService::initialize(Arc::clone(&storage)).unwrap();

        service.pin(first.to_string_lossy().into_owned(), Some("测试".to_owned())).unwrap();
        service.pin(second.to_string_lossy().into_owned(), Some("测试".to_owned())).unwrap();
        let reordered = service.reorder(&second.to_string_lossy(), &first.to_string_lossy()).unwrap();
        assert_eq!(reordered.pinned_apps[0].name, "Second");
        let hidden = service.set_visible(&second.to_string_lossy(), false).unwrap();
        assert!(!hidden.pinned_apps[0].visible);

        let reopened = QuickLaunchService::initialize(storage).unwrap();
        assert_eq!(reopened.snapshot().unwrap().pinned_apps.len(), 2);
        let remaining = reopened.unpin(&first.to_string_lossy()).unwrap();
        assert_eq!(remaining.pinned_apps.len(), 1);
        assert_eq!(remaining.pinned_apps[0].name, "Second");
    }

    #[cfg(windows)]
    #[test]
    fn fixed_executable_exposes_a_real_cached_shell_icon() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        let service = QuickLaunchService::initialize(storage).unwrap();
        let executable = std::env::current_exe().unwrap();
        let executable = executable.to_string_lossy().into_owned();

        service.pin(executable.clone(), Some("测试".to_owned())).unwrap();
        let icon = service.icon_bytes(&executable).unwrap();
        assert!(icon.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
