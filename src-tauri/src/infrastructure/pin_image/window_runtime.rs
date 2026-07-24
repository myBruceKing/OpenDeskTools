#[cfg(windows)]
mod windows_impl;

#[cfg(windows)]
pub use windows_impl::{
    render_text_card, NativeImageSurfaceRuntime, NativeSurfaceError, PinImageData,
};

#[cfg(not(windows))]
mod unsupported {
    use std::sync::Arc;

    use thiserror::Error;

    use crate::infrastructure::clipboard_listener::ClipboardSequenceSuppressor;
    use crate::infrastructure::keyboard_hook::KeyboardHookBroker;

    #[derive(Debug)]
    pub struct NativeImageSurfaceRuntime;

    #[derive(Debug)]
    pub struct PinImageData {
        pub width: u32,
        pub height: u32,
        pub rgba: Vec<u8>,
        pub source_text: Option<String>,
    }

    #[derive(Debug, Error)]
    pub enum NativeSurfaceError {
        #[error("native image surfaces are only available on Windows")]
        UnsupportedPlatform,
        #[error("native image surface startup failed: {0}")]
        Startup(String),
    }

    impl NativeImageSurfaceRuntime {
        pub fn start(
            _keyboard_hook: Arc<KeyboardHookBroker>,
            _suppressor: ClipboardSequenceSuppressor,
        ) -> Result<Self, NativeSurfaceError> {
            Err(NativeSurfaceError::UnsupportedPlatform)
        }

        pub fn open(&self, _image: PinImageData) -> Result<u64, NativeSurfaceError> {
            Err(NativeSurfaceError::UnsupportedPlatform)
        }
    }

    pub fn render_text_card(_text: &str) -> Result<PinImageData, NativeSurfaceError> {
        Err(NativeSurfaceError::UnsupportedPlatform)
    }
}

#[cfg(not(windows))]
pub use unsupported::{
    render_text_card, NativeImageSurfaceRuntime, NativeSurfaceError, PinImageData,
};
