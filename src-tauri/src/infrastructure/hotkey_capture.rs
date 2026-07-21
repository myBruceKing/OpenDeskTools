use std::sync::{Arc, Mutex};

use serde::Serialize;
use thiserror::Error;

use super::keyboard_hook::{KeyboardHookBroker, KeyboardHookError};

const SESSION_ID_PREFIX: &str = "hotkey-capture-";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyCaptureSession {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyCaptureStopResult {
    pub session_id: String,
    pub stopped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyCaptureEvent {
    pub session_id: String,
    pub token: String,
}

#[derive(Debug, Error)]
pub enum HotkeyCaptureError {
    #[error(transparent)]
    Hook(#[from] KeyboardHookError),
    #[error("hotkey capture state lock is poisoned")]
    StateLockPoisoned,
}

#[derive(Debug, Clone)]
pub struct HotkeyCaptureManager {
    broker: Arc<KeyboardHookBroker>,
    active_session: Arc<Mutex<Option<u64>>>,
}

impl HotkeyCaptureManager {
    pub fn new(broker: Arc<KeyboardHookBroker>) -> Self {
        Self {
            broker,
            active_session: Arc::new(Mutex::new(None)),
        }
    }

    pub fn start<F>(
        &self,
        target_window: usize,
        event_sink: F,
    ) -> Result<HotkeyCaptureSession, HotkeyCaptureError>
    where
        F: Fn(HotkeyCaptureEvent) + Send + Sync + 'static,
    {
        self.stop_active()?;
        let session = self
            .broker
            .start_capture(target_window, move |session_id, token| {
                event_sink(HotkeyCaptureEvent {
                    session_id: format!("{SESSION_ID_PREFIX}{session_id}"),
                    token,
                });
            })?;
        *self
            .active_session
            .lock()
            .map_err(|_| HotkeyCaptureError::StateLockPoisoned)? = Some(session);
        Ok(HotkeyCaptureSession {
            session_id: format!("{SESSION_ID_PREFIX}{session}"),
        })
    }

    pub fn stop(&self, session_id: &str) -> Result<HotkeyCaptureStopResult, HotkeyCaptureError> {
        let numeric = parse_session_id(session_id);
        let mut active = self
            .active_session
            .lock()
            .map_err(|_| HotkeyCaptureError::StateLockPoisoned)?;
        if numeric.is_none() || *active != numeric {
            return Ok(HotkeyCaptureStopResult {
                session_id: session_id.to_owned(),
                stopped: false,
            });
        }
        let numeric = numeric.expect("matching numeric session");
        let stopped = self.broker.stop_capture(numeric)?;
        if stopped {
            *active = None;
        }
        Ok(HotkeyCaptureStopResult {
            session_id: session_id.to_owned(),
            stopped,
        })
    }

    pub fn stop_active(&self) -> Result<(), HotkeyCaptureError> {
        self.broker.stop_active_capture()?;
        *self
            .active_session
            .lock()
            .map_err(|_| HotkeyCaptureError::StateLockPoisoned)? = None;
        Ok(())
    }
}

fn parse_session_id(value: &str) -> Option<u64> {
    value.strip_prefix(SESSION_ID_PREFIX)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_are_canonical_and_stale_ids_do_not_match() {
        assert_eq!(parse_session_id("hotkey-capture-42"), Some(42));
        assert_eq!(parse_session_id("hotkey-capture-01"), Some(1));
        assert_eq!(parse_session_id("capture-42"), None);
    }

    #[test]
    fn stopping_an_absent_session_is_idempotent() {
        let manager = HotkeyCaptureManager::new(Arc::new(KeyboardHookBroker::default()));
        let result = manager.stop("hotkey-capture-1").unwrap();
        assert_eq!(
            result,
            HotkeyCaptureStopResult {
                session_id: "hotkey-capture-1".to_owned(),
                stopped: false,
            }
        );
    }
}
