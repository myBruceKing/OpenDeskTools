use tauri::{LogicalSize, Runtime, WebviewWindow};

pub const MAIN_WEBVIEW_LABEL: &str = "main";
const DEFAULT_WIDTH: u32 = 1080;
const DEFAULT_HEIGHT: u32 = 720;
const MIN_WIDTH: u32 = 960;
const MIN_HEIGHT: u32 = 640;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InitialWindowSize {
    width: u32,
    height: u32,
}

pub fn configure_main_window<R: Runtime>(window: &WebviewWindow<R>) -> tauri::Result<()> {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    let work_area_logical_size = monitor.map(|monitor| {
        let logical = monitor
            .work_area()
            .size
            .to_logical::<f64>(monitor.scale_factor());
        (logical.width, logical.height)
    });
    let initial_size = calculate_initial_window_size(work_area_logical_size);

    window.set_min_size(Some(LogicalSize::new(MIN_WIDTH as f64, MIN_HEIGHT as f64)))?;
    window.set_size(LogicalSize::new(
        initial_size.width as f64,
        initial_size.height as f64,
    ))?;
    window.center()?;
    configure_window_shape(window)?;

    Ok(())
}

#[cfg(windows)]
fn configure_window_shape<R: Runtime>(window: &WebviewWindow<R>) -> tauri::Result<()> {
    apply_native_corner_preference(window);

    Ok(())
}

#[cfg(not(windows))]
fn configure_window_shape<R: Runtime>(_window: &WebviewWindow<R>) -> tauri::Result<()> {
    Ok(())
}

#[cfg(windows)]
fn apply_native_corner_preference<R: Runtime>(window: &WebviewWindow<R>) {
    use std::mem::size_of_val;
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    };

    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let preference = DWMWCP_ROUND;

    unsafe {
        DwmSetWindowAttribute(
            hwnd.0,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &preference as *const _ as *const core::ffi::c_void,
            size_of_val(&preference) as u32,
        );
    };
}

fn calculate_initial_window_size(work_area_logical_size: Option<(f64, f64)>) -> InitialWindowSize {
    let Some((work_area_width, work_area_height)) = work_area_logical_size else {
        return InitialWindowSize {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
        };
    };

    InitialWindowSize {
        width: fit_dimension(work_area_width, MIN_WIDTH, DEFAULT_WIDTH),
        height: fit_dimension(work_area_height, MIN_HEIGHT, DEFAULT_HEIGHT),
    }
}

fn fit_dimension(work_area_dimension: f64, minimum: u32, default: u32) -> u32 {
    (work_area_dimension.floor() as u32).clamp(minimum, default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_default_size_without_a_monitor() {
        assert_eq!(
            calculate_initial_window_size(None),
            InitialWindowSize {
                width: DEFAULT_WIDTH,
                height: DEFAULT_HEIGHT,
            }
        );
    }

    #[test]
    fn uses_fixed_default_when_the_work_area_is_large_enough() {
        assert_eq!(
            calculate_initial_window_size(Some((1366.0, 728.0))),
            InitialWindowSize {
                width: DEFAULT_WIDTH,
                height: DEFAULT_HEIGHT,
            }
        );
        assert_eq!(
            calculate_initial_window_size(Some((1920.0, 1040.0))),
            InitialWindowSize {
                width: DEFAULT_WIDTH,
                height: DEFAULT_HEIGHT,
            }
        );
    }

    #[test]
    fn safely_fits_inside_a_smaller_work_area() {
        assert_eq!(
            calculate_initial_window_size(Some((1000.8, 680.9))),
            InitialWindowSize {
                width: 1000,
                height: 680,
            }
        );
        assert_eq!(
            calculate_initial_window_size(Some((800.0, 600.0))),
            InitialWindowSize {
                width: MIN_WIDTH,
                height: MIN_HEIGHT,
            }
        );
    }

    #[test]
    fn never_returns_less_than_the_supported_minimum() {
        assert_eq!(
            calculate_initial_window_size(Some((0.0, 0.0))),
            InitialWindowSize {
                width: MIN_WIDTH,
                height: MIN_HEIGHT,
            }
        );
    }
}
