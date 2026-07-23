use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, Runtime, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use thiserror::Error;

use super::{popup_geometry::top_right_position, surface_window_animation};

pub const QR_TOAST_SURFACE_LABEL: &str = "qr-toast-surface";
pub const QR_CONVERSION_EVENT: &str = "qr://conversion-result";
const QR_TOAST_SURFACE_ROUTE: &str = "index.html#qr-toast-surface";
const QR_TOAST_WIDTH: f64 = 390.0;
const QR_TOAST_HEIGHT: f64 = 88.0;
const QR_TOAST_EDGE_GAP: i32 = 16;
const QR_TOAST_SUCCESS_DWELL_MS: u64 = 3_200;
const QR_TOAST_ERROR_DWELL_MS: u64 = 4_800;
static QR_TOAST_GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum QrToastSurfaceError {
    #[error("the QR feedback surface is not prepared")]
    NotPrepared,
    #[error("the QR feedback surface has no available monitor")]
    MonitorUnavailable,
    #[error("the QR feedback surface dimensions are invalid")]
    InvalidDimensions,
    #[error("QR feedback window operation failed: {0}")]
    Window(#[from] tauri::Error),
    #[error("QR feedback could not be serialized")]
    Serialization,
}

pub fn prepare<R: Runtime>(app: &AppHandle<R>) -> Result<(), QrToastSurfaceError> {
    if app.get_webview_window(QR_TOAST_SURFACE_LABEL).is_some() {
        return Ok(());
    }
    WebviewWindowBuilder::new(
        app,
        QR_TOAST_SURFACE_LABEL,
        WebviewUrl::App(QR_TOAST_SURFACE_ROUTE.into()),
    )
    .title("")
    .inner_size(QR_TOAST_WIDTH, QR_TOAST_HEIGHT)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .transparent(true)
    .focusable(false)
    .visible(false)
    .build()?;
    Ok(())
}

pub fn show<R: Runtime>(app: &AppHandle<R>, payload: &Value) -> Result<(), QrToastSurfaceError> {
    let window = app
        .get_webview_window(QR_TOAST_SURFACE_LABEL)
        .ok_or(QrToastSurfaceError::NotPrepared)?;
    place_at_primary_work_area_top_right(&window)?;
    surface_window_animation::prepare_show(&window);
    publish_feedback(&window, payload)?;
    window.show()?;

    let generation = QR_TOAST_GENERATION
        .fetch_add(1, Ordering::SeqCst)
        .wrapping_add(1);
    let dwell_ms = if payload
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        QR_TOAST_SUCCESS_DWELL_MS
    } else {
        QR_TOAST_ERROR_DWELL_MS
    };
    let delayed_window = window.clone();
    let delayed_app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(dwell_ms));
        if QR_TOAST_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        let main_thread_window = delayed_window.clone();
        if delayed_app
            .run_on_main_thread(move || {
                if QR_TOAST_GENERATION.load(Ordering::SeqCst) == generation {
                    let _ = surface_window_animation::fade_hide(&main_thread_window);
                }
            })
            .is_err()
        {
            let _ = delayed_window.hide();
        }
    });
    Ok(())
}

fn publish_feedback<R: Runtime>(
    window: &WebviewWindow<R>,
    payload: &Value,
) -> Result<(), QrToastSurfaceError> {
    window.emit(QR_CONVERSION_EVENT, payload)?;
    let serialized =
        serde_json::to_string(payload).map_err(|_| QrToastSurfaceError::Serialization)?;
    window.eval(format!(
        "window.__OPENDESK_QR_FEEDBACK={serialized};window.dispatchEvent(new Event('opendesk-qr-feedback'));"
    ))?;
    Ok(())
}

fn place_at_primary_work_area_top_right<R: Runtime>(
    window: &WebviewWindow<R>,
) -> Result<(), QrToastSurfaceError> {
    let monitor = window
        .primary_monitor()?
        .ok_or(QrToastSurfaceError::MonitorUnavailable)?;
    let work = monitor.work_area();
    let size = window.outer_size()?;
    let width = i32::try_from(size.width).map_err(|_| QrToastSurfaceError::InvalidDimensions)?;
    let height = i32::try_from(size.height).map_err(|_| QrToastSurfaceError::InvalidDimensions)?;
    let (x, y) = top_right_position(
        (work.position.x, work.position.y),
        (work.size.width, work.size.height),
        (width, height),
        QR_TOAST_EDGE_GAP,
    )
    .ok_or(QrToastSurfaceError::InvalidDimensions)?;
    window.set_position(PhysicalPosition::new(x, y))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_feedback_surface_is_declared_in_the_default_capability() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../../capabilities/default.json"))
                .expect("default capability should be valid JSON");
        assert!(capability["windows"]
            .as_array()
            .expect("default capability should declare windows")
            .iter()
            .any(|label| label.as_str() == Some(QR_TOAST_SURFACE_LABEL)));
    }
}
