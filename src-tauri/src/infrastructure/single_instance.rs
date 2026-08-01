//! Process-level single-instance coordination that runs before Tauri creates
//! its Windows event loop.
//!
//! Elevated autostart deliberately allows a short-lived parallel task process
//! so a normal desktop launch can wake the already elevated primary process.
//! That peer must not initialize and immediately tear down Tauri: doing so can
//! race TAO's Windows event-loop destruction. A named mutex elects the primary,
//! while a named auto-reset event carries wake requests without creating any
//! WebView or Tauri window in the secondary process.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use tauri::{AppHandle, Manager, Runtime};
use thiserror::Error;

const MUTEX_NAME: &str = r"Local\OpenDeskTools.SingleInstance.Primary";
const WAKE_EVENT_NAME: &str = r"Local\OpenDeskTools.SingleInstance.Wake";

#[derive(Debug, Error)]
pub enum SingleInstanceError {
    #[error("failed to create the single-instance wake event: {0}")]
    CreateWakeEvent(#[source] io::Error),
    #[error("failed to create the single-instance mutex: {0}")]
    CreateMutex(#[source] io::Error),
    #[error("failed to notify the primary OpenDeskTools process: {0}")]
    NotifyPrimary(#[source] io::Error),
    #[error("failed to start the single-instance wake listener: {0}")]
    StartListener(#[source] io::Error),
}

#[derive(Debug)]
pub enum InstanceClaim {
    Primary(PrimaryInstance),
    SecondaryNotified,
}

#[derive(Debug)]
pub struct PrimaryInstance {
    platform: platform::PrimaryHandles,
}

impl PrimaryInstance {
    pub fn start_listener<R: Runtime>(
        self,
        app: &AppHandle<R>,
    ) -> Result<SingleInstanceManager, SingleInstanceError> {
        let stopping = Arc::new(AtomicBool::new(false));
        let listener_stopping = Arc::clone(&stopping);
        let wake_event = self.platform.wake_event();
        let listener_app = app.clone();
        let listener = thread::Builder::new()
            .name("single-instance-wake".to_owned())
            .spawn(move || loop {
                match platform::wait_for_wake(wake_event) {
                    platform::WakeWait::Signaled => {
                        if listener_stopping.load(Ordering::Acquire) {
                            break;
                        }
                        let main_thread_app = listener_app.clone();
                        let main_thread_stopping = Arc::clone(&listener_stopping);
                        if listener_app
                            .run_on_main_thread(move || {
                                if main_thread_stopping.load(Ordering::Acquire) {
                                    return;
                                }
                                if let Err(error) =
                                    super::tray::open_main_window(&main_thread_app)
                                {
                                    eprintln!(
                                        "failed to wake the main window from a repeated launch: {error}"
                                    );
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    platform::WakeWait::TimedOut => {
                        if listener_stopping.load(Ordering::Acquire) {
                            break;
                        }
                    }
                    platform::WakeWait::Failed(error) => {
                        eprintln!("single-instance wake listener stopped: {error}");
                        break;
                    }
                }
            })
            .map_err(SingleInstanceError::StartListener)?;
        Ok(SingleInstanceManager {
            primary: self,
            stopping,
            listener: Mutex::new(Some(listener)),
        })
    }
}

#[derive(Debug)]
pub struct SingleInstanceManager {
    primary: PrimaryInstance,
    stopping: Arc<AtomicBool>,
    listener: Mutex<Option<JoinHandle<()>>>,
}

impl SingleInstanceManager {
    /// Stops accepting wake requests before Tauri begins event-loop teardown.
    /// The primary mutex remains held until the process itself exits.
    pub fn stop(&self) {
        if self.stopping.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Err(error) = self.primary.platform.signal_wake() {
            eprintln!("failed to stop the single-instance wake listener: {error}");
        }
        let listener = self
            .listener
            .lock()
            .ok()
            .and_then(|mut listener| listener.take());
        if let Some(listener) = listener {
            let _ = listener.join();
        }
    }
}

impl Drop for SingleInstanceManager {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn claim() -> Result<InstanceClaim, SingleInstanceError> {
    claim_named(MUTEX_NAME, WAKE_EVENT_NAME)
}

fn claim_named(
    mutex_name: &str,
    wake_event_name: &str,
) -> Result<InstanceClaim, SingleInstanceError> {
    match platform::claim(mutex_name, wake_event_name)? {
        platform::PlatformClaim::Primary(platform) => {
            Ok(InstanceClaim::Primary(PrimaryInstance { platform }))
        }
        platform::PlatformClaim::SecondaryNotified => Ok(InstanceClaim::SecondaryNotified),
    }
}

pub fn stop_listener<R: Runtime>(app: &AppHandle<R>) {
    if let Some(manager) = app.try_state::<SingleInstanceManager>() {
        manager.stop();
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::OsStr;
    use std::io;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Threading::{
        CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject,
    };

    use super::SingleInstanceError;

    const WAKE_POLL_INTERVAL_MS: u32 = 500;

    #[derive(Debug)]
    struct OwnedHandle(isize);

    impl OwnedHandle {
        fn raw(&self) -> HANDLE {
            self.0 as HANDLE
        }
    }

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            if self.0 != 0 {
                unsafe {
                    CloseHandle(self.0 as HANDLE);
                }
            }
        }
    }

    #[derive(Debug)]
    pub struct PrimaryHandles {
        _mutex: OwnedHandle,
        wake_event: OwnedHandle,
    }

    impl PrimaryHandles {
        pub fn wake_event(&self) -> isize {
            self.wake_event.raw() as isize
        }

        pub fn signal_wake(&self) -> io::Result<()> {
            signal(self.wake_event.raw())
        }
    }

    pub enum PlatformClaim {
        Primary(PrimaryHandles),
        SecondaryNotified,
    }

    pub enum WakeWait {
        Signaled,
        TimedOut,
        Failed(io::Error),
    }

    pub fn claim(
        mutex_name: &str,
        wake_event_name: &str,
    ) -> Result<PlatformClaim, SingleInstanceError> {
        let wake_event_name = wide_null(wake_event_name);
        let wake_event = unsafe { CreateEventW(std::ptr::null(), 0, 0, wake_event_name.as_ptr()) };
        if wake_event.is_null() {
            return Err(SingleInstanceError::CreateWakeEvent(
                io::Error::last_os_error(),
            ));
        }
        let wake_event = OwnedHandle(wake_event as isize);

        let mutex_name = wide_null(mutex_name);
        let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
        if mutex.is_null() {
            return Err(SingleInstanceError::CreateMutex(io::Error::last_os_error()));
        }
        let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let mutex = OwnedHandle(mutex as isize);

        if already_exists {
            signal(wake_event.raw()).map_err(SingleInstanceError::NotifyPrimary)?;
            Ok(PlatformClaim::SecondaryNotified)
        } else {
            Ok(PlatformClaim::Primary(PrimaryHandles {
                _mutex: mutex,
                wake_event,
            }))
        }
    }

    pub fn wait_for_wake(wake_event: isize) -> WakeWait {
        let result = unsafe { WaitForSingleObject(wake_event as HANDLE, WAKE_POLL_INTERVAL_MS) };
        match result {
            WAIT_OBJECT_0 => WakeWait::Signaled,
            WAIT_TIMEOUT => WakeWait::TimedOut,
            _ => WakeWait::Failed(io::Error::last_os_error()),
        }
    }

    fn signal(wake_event: HANDLE) -> io::Result<()> {
        if unsafe { SetEvent(wake_event) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn wide_null(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain(Some(0)).collect()
    }
}

#[cfg(not(windows))]
mod platform {
    use std::io;

    use super::SingleInstanceError;

    #[derive(Debug)]
    pub struct PrimaryHandles;

    impl PrimaryHandles {
        pub fn wake_event(&self) -> isize {
            0
        }

        pub fn signal_wake(&self) -> io::Result<()> {
            Ok(())
        }
    }

    pub enum PlatformClaim {
        Primary(PrimaryHandles),
        SecondaryNotified,
    }

    pub enum WakeWait {
        Signaled,
        TimedOut,
        Failed(io::Error),
    }

    pub fn claim(
        _mutex_name: &str,
        _wake_event_name: &str,
    ) -> Result<PlatformClaim, SingleInstanceError> {
        Ok(PlatformClaim::Primary(PrimaryHandles))
    }

    pub fn wait_for_wake(_wake_event: isize) -> WakeWait {
        WakeWait::TimedOut
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::Threading::WaitForSingleObject;

    use super::*;

    static NEXT_NAME: AtomicU64 = AtomicU64::new(1);

    fn unique_names() -> (String, String) {
        let suffix = NEXT_NAME.fetch_add(1, Ordering::Relaxed);
        (
            format!(
                r"Local\OpenDeskTools.Test.{}.{}.Mutex",
                std::process::id(),
                suffix
            ),
            format!(
                r"Local\OpenDeskTools.Test.{}.{}.Wake",
                std::process::id(),
                suffix
            ),
        )
    }

    #[test]
    fn repeated_claim_notifies_primary_without_starting_tauri() {
        let (mutex_name, event_name) = unique_names();
        let primary = match claim_named(&mutex_name, &event_name).expect("primary claim") {
            InstanceClaim::Primary(primary) => primary,
            InstanceClaim::SecondaryNotified => panic!("first claimant must be primary"),
        };

        assert!(matches!(
            claim_named(&mutex_name, &event_name).expect("secondary claim"),
            InstanceClaim::SecondaryNotified
        ));
        assert_eq!(
            unsafe { WaitForSingleObject(primary.platform.wake_event() as _, 0) },
            WAIT_OBJECT_0
        );
    }

    #[test]
    fn closing_primary_releases_the_named_instance() {
        let (mutex_name, event_name) = unique_names();
        let primary = match claim_named(&mutex_name, &event_name).expect("primary claim") {
            InstanceClaim::Primary(primary) => primary,
            InstanceClaim::SecondaryNotified => panic!("first claimant must be primary"),
        };
        drop(primary);

        assert!(matches!(
            claim_named(&mutex_name, &event_name).expect("replacement primary claim"),
            InstanceClaim::Primary(_)
        ));
    }
}
