use std::sync::Arc;

use thiserror::Error;

use super::clipboard::{ClipboardError, ClipboardService, ClipboardWriteContent};
use super::clipboard_writer::{ClipboardWriter, ClipboardWriterError};
use super::debug_qa;
use super::surface::{SurfaceError, SurfaceInputTargetRequirement, SurfaceManager};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardActionKind {
    Copied,
    Input,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardActionOutcome {
    pub action: ClipboardActionKind,
    pub clipboard_updated: bool,
}

#[derive(Debug, Error)]
pub enum ClipboardInputError {
    #[error("clipboard history operation failed")]
    Clipboard(#[from] ClipboardError),
    #[error("clipboard write operation failed")]
    Writer(#[from] ClipboardWriterError),
    #[error("clipboard surface target operation failed")]
    Surface(#[from] SurfaceError),
    #[error("clipboard changed before the guarded write")]
    ClipboardChanged,
    #[error("a modifier key is currently pressed")]
    ModifierPressed,
    #[error("Windows denied synthetic paste input")]
    InputDenied,
    #[error("Windows denied cleanup of partially inserted synthetic input")]
    InputCleanupDenied,
}

#[derive(Debug)]
pub struct ClipboardInputCoordinator {
    clipboard: Arc<ClipboardService>,
    writer: ClipboardWriter,
    surface: Arc<SurfaceManager>,
}

impl ClipboardInputCoordinator {
    pub fn new(clipboard: Arc<ClipboardService>, surface: Arc<SurfaceManager>) -> Self {
        Self {
            clipboard,
            writer: ClipboardWriter::default(),
            surface,
        }
    }

    pub fn copy<F>(
        &self,
        id: i64,
        owner_window: usize,
        mut suppress: F,
    ) -> Result<ClipboardActionOutcome, ClipboardInputError>
    where
        F: FnMut(u32),
    {
        let content = self.clipboard.content_for_write(id)?;
        self.writer.transaction(owner_window, |transaction| {
            match transaction.replace_current(&content, &mut suppress)? {
                Some(_) => Ok(ClipboardActionOutcome {
                    action: ClipboardActionKind::Copied,
                    clipboard_updated: true,
                }),
                None => Err(ClipboardInputError::ClipboardChanged),
            }
        })
    }

    pub fn input<F>(
        &self,
        id: i64,
        owner_window: usize,
        mut suppress: F,
    ) -> Result<ClipboardActionOutcome, ClipboardInputError>
    where
        F: FnMut(u32),
    {
        let content = self.clipboard.content_for_write(id)?;
        let requirement = input_target_requirement(&content);
        debug_qa::trace(format!(
            "clipboard input begin id={id} content_kind={} target_requirement={}",
            content_kind(&content),
            requirement.as_str()
        ));
        let handoff = self.surface.begin_input_handoff_for(requirement)?;
        let restored_target = self.surface.restore_and_run(&handoff, || {
            self.writer.transaction(owner_window, |transaction| {
                transaction
                    .replace_current(&content, &mut suppress)?
                    .ok_or(ClipboardInputError::ClipboardChanged)?;
                let input_result = send_ctrl_v(&mut SystemInputApi);
                debug_qa::trace(format!(
                    "clipboard input keys_injected id={id} result={}",
                    if input_result.is_ok() { "ok" } else { "error" }
                ));
                input_result
            })
        });
        let result = match restored_target {
            Ok((generation, input_result)) => {
                input_result?;
                // A newer Win+V capture owns a different generation and must not
                // be cleared when this older request finishes.
                self.surface.clear_if_generation(generation);
                Ok(ClipboardActionOutcome {
                    action: ClipboardActionKind::Input,
                    clipboard_updated: true,
                })
            }
            Err(error) => Err(ClipboardInputError::Surface(error)),
        };
        drop(handoff);
        result
    }
}

fn input_target_requirement(content: &ClipboardWriteContent) -> SurfaceInputTargetRequirement {
    match content {
        ClipboardWriteContent::Files { .. } => SurfaceInputTargetRequirement::FocusedDescendant,
        ClipboardWriteContent::Text(_) | ClipboardWriteContent::Image { .. } => {
            SurfaceInputTargetRequirement::ActiveTarget
        }
    }
}

fn content_kind(content: &ClipboardWriteContent) -> &'static str {
    match content {
        ClipboardWriteContent::Text(_) => "text",
        ClipboardWriteContent::Image { .. } => "image",
        ClipboardWriteContent::Files { .. } => "files",
    }
}

trait InputApi {
    fn modifier_pressed(&mut self) -> bool;
    fn send(&mut self, inputs: &[KeyInput]) -> usize;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyInput {
    virtual_key: u16,
    key_up: bool,
}

fn send_ctrl_v<A: InputApi>(api: &mut A) -> Result<(), ClipboardInputError> {
    if api.modifier_pressed() {
        return Err(ClipboardInputError::ModifierPressed);
    }
    let inputs = [
        KeyInput {
            virtual_key: 0x11,
            key_up: false,
        },
        KeyInput {
            virtual_key: 0x56,
            key_up: false,
        },
        KeyInput {
            virtual_key: 0x56,
            key_up: true,
        },
        KeyInput {
            virtual_key: 0x11,
            key_up: true,
        },
    ];
    let inserted = api.send(&inputs);
    if inserted == inputs.len() {
        return Ok(());
    }
    let mut releases = Vec::new();
    if inserted >= 2 {
        releases.push(KeyInput {
            virtual_key: 0x56,
            key_up: true,
        });
    }
    if inserted >= 1 {
        releases.push(KeyInput {
            virtual_key: 0x11,
            key_up: true,
        });
    }
    if !releases.is_empty() {
        let cleanup_inserted = api.send(&releases);
        if cleanup_inserted != releases.len() {
            let mut all_released = true;
            for release in &releases {
                all_released &= api.send(&[*release]) == 1;
            }
            if !all_released {
                return Err(ClipboardInputError::InputCleanupDenied);
            }
        }
    }
    Err(ClipboardInputError::InputDenied)
}

struct SystemInputApi;

#[cfg(windows)]
impl InputApi for SystemInputApi {
    fn modifier_pressed(&mut self) -> bool {
        [0x10_i32, 0x11, 0x12, 0x5b, 0x5c]
            .into_iter()
            .any(|key| unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(key) } < 0)
    }
    fn send(&mut self, inputs: &[KeyInput]) -> usize {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
        };
        let native = inputs
            .iter()
            .map(|input| {
                let mut native: INPUT = unsafe { std::mem::zeroed() };
                native.r#type = INPUT_KEYBOARD;
                native.Anonymous.ki = KEYBDINPUT {
                    wVk: input.virtual_key,
                    wScan: 0,
                    dwFlags: if input.key_up { KEYEVENTF_KEYUP } else { 0 },
                    time: 0,
                    dwExtraInfo: 0,
                };
                native
            })
            .collect::<Vec<_>>();
        usize::try_from(unsafe {
            SendInput(
                native.len() as u32,
                native.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        })
        .unwrap_or(0)
    }
}

#[cfg(not(windows))]
impl InputApi for SystemInputApi {
    fn modifier_pressed(&mut self) -> bool {
        false
    }
    fn send(&mut self, _inputs: &[KeyInput]) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_require_a_focused_descendant_but_text_and_images_do_not() {
        assert_eq!(
            input_target_requirement(&ClipboardWriteContent::Files {
                paths: vec![vec![b'C' as u16]],
            }),
            SurfaceInputTargetRequirement::FocusedDescendant
        );
        assert_eq!(
            input_target_requirement(&ClipboardWriteContent::Text("text".to_owned())),
            SurfaceInputTargetRequirement::ActiveTarget
        );
        assert_eq!(
            input_target_requirement(&ClipboardWriteContent::Image {
                width: 1,
                height: 1,
                rgba: vec![0, 0, 0, 0],
            }),
            SurfaceInputTargetRequirement::ActiveTarget
        );
    }

    struct FakeInput {
        modifier: bool,
        results: Vec<usize>,
        calls: Vec<Vec<KeyInput>>,
    }
    impl InputApi for FakeInput {
        fn modifier_pressed(&mut self) -> bool {
            self.modifier
        }
        fn send(&mut self, inputs: &[KeyInput]) -> usize {
            self.calls.push(inputs.to_vec());
            self.results.remove(0)
        }
    }
    #[test]
    fn modifier_state_rejects_without_input_and_partial_send_releases_pressed_keys() {
        let mut modifier = FakeInput {
            modifier: true,
            results: vec![],
            calls: vec![],
        };
        assert!(matches!(
            send_ctrl_v(&mut modifier),
            Err(ClipboardInputError::ModifierPressed)
        ));
        assert!(modifier.calls.is_empty());
        let mut partial = FakeInput {
            modifier: false,
            results: vec![2, 2],
            calls: vec![],
        };
        assert!(matches!(
            send_ctrl_v(&mut partial),
            Err(ClipboardInputError::InputDenied)
        ));
        assert_eq!(
            partial.calls[1],
            vec![
                KeyInput {
                    virtual_key: 0x56,
                    key_up: true
                },
                KeyInput {
                    virtual_key: 0x11,
                    key_up: true
                }
            ]
        );
    }
    #[test]
    fn complete_ctrl_v_sequence_is_inserted_in_order() {
        let mut api = FakeInput {
            modifier: false,
            results: vec![4],
            calls: vec![],
        };
        send_ctrl_v(&mut api).unwrap();
        assert_eq!(
            api.calls[0][0],
            KeyInput {
                virtual_key: 0x11,
                key_up: false
            }
        );
        assert_eq!(
            api.calls[0][3],
            KeyInput {
                virtual_key: 0x11,
                key_up: true
            }
        );
    }

    #[test]
    fn every_partial_prefix_is_cleaned_and_cleanup_denial_is_explicit() {
        for inserted in 0..4 {
            let cleanup_len = match inserted {
                0 => 0,
                1 => 1,
                _ => 2,
            };
            let mut results = vec![inserted];
            if cleanup_len > 0 {
                results.push(cleanup_len);
            }
            let mut api = FakeInput {
                modifier: false,
                results,
                calls: vec![],
            };
            assert!(matches!(
                send_ctrl_v(&mut api),
                Err(ClipboardInputError::InputDenied)
            ));
        }

        let mut denied = FakeInput {
            modifier: false,
            // Ctrl-down inserted; batch Ctrl-up and single Ctrl-up both denied.
            results: vec![1, 0, 0],
            calls: vec![],
        };
        assert!(matches!(
            send_ctrl_v(&mut denied),
            Err(ClipboardInputError::InputCleanupDenied)
        ));

        let mut continues_after_first_release_failure = FakeInput {
            modifier: false,
            results: vec![2, 0, 0, 1],
            calls: vec![],
        };
        assert!(matches!(
            send_ctrl_v(&mut continues_after_first_release_failure),
            Err(ClipboardInputError::InputCleanupDenied)
        ));
        assert_eq!(continues_after_first_release_failure.calls.len(), 4);
        assert_eq!(
            continues_after_first_release_failure.calls[3],
            vec![KeyInput {
                virtual_key: 0x11,
                key_up: true
            }]
        );
    }
}
