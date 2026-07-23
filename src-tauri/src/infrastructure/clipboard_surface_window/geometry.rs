use tauri::Monitor;

use super::{
    CLIPBOARD_PREVIEW_SURFACE_HEIGHT, CLIPBOARD_PREVIEW_SURFACE_WIDTH,
    CLIPBOARD_SURFACE_CURSOR_GAP, CLIPBOARD_SURFACE_HEIGHT, CLIPBOARD_SURFACE_WIDTH,
};
use crate::infrastructure::popup_geometry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PixelPoint {
    pub(super) x: i32,
    pub(super) y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PixelSize {
    pub(super) width: u32,
    pub(super) height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PixelRect {
    pub(super) left: i32,
    pub(super) top: i32,
    pub(super) right: i32,
    pub(super) bottom: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct MonitorGeometry {
    pub(super) bounds: PixelRect,
    pub(super) work_area: PixelRect,
    pub(super) scale_factor: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SurfacePlacement {
    pub(super) position: PixelPoint,
    pub(super) size: PixelSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SurfaceAnchorSource {
    Caret,
    Cursor,
}

impl SurfaceAnchorSource {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Caret => "caret",
            Self::Cursor => "cursor",
        }
    }
}

pub(super) fn monitor_geometry(monitor: &Monitor) -> Option<MonitorGeometry> {
    let bounds = rect_from_origin_size(
        monitor.position().x,
        monitor.position().y,
        monitor.size().width,
        monitor.size().height,
    )?;
    let work = monitor.work_area();
    let work_area = rect_from_origin_size(
        work.position.x,
        work.position.y,
        work.size.width,
        work.size.height,
    )?;
    PixelRect::is_valid(bounds)
        .then_some(())
        .and_then(|_| PixelRect::is_valid(work_area).then_some(()))?;
    let scale_factor = monitor.scale_factor();
    (scale_factor.is_finite() && scale_factor > 0.0).then_some(MonitorGeometry {
        bounds,
        work_area,
        scale_factor,
    })
}

pub(super) fn rect_from_origin_size(x: i32, y: i32, width: u32, height: u32) -> Option<PixelRect> {
    let right = i64::from(x).checked_add(i64::from(width))?;
    let bottom = i64::from(y).checked_add(i64::from(height))?;
    Some(PixelRect {
        left: x,
        top: y,
        right: i32::try_from(right).ok()?,
        bottom: i32::try_from(bottom).ok()?,
    })
}

impl PixelRect {
    pub(super) fn is_valid(self) -> bool {
        self.right > self.left && self.bottom > self.top
    }

    pub(super) fn contains(self, point: PixelPoint) -> bool {
        popup_geometry::point_in_rect(
            (point.x, point.y),
            (self.left, self.top),
            (self.width(), self.height()),
        )
    }

    pub(super) fn width(self) -> u32 {
        u32::try_from(i64::from(self.right) - i64::from(self.left)).unwrap_or(0)
    }

    pub(super) fn height(self) -> u32 {
        u32::try_from(i64::from(self.bottom) - i64::from(self.top)).unwrap_or(0)
    }
}

pub(super) fn select_anchor_monitor(
    anchor: PixelPoint,
    monitors: &[MonitorGeometry],
) -> Option<MonitorGeometry> {
    monitors
        .iter()
        .copied()
        .find(|monitor| monitor.bounds.contains(anchor))
        .or_else(|| {
            monitors.iter().copied().min_by_key(|monitor| {
                popup_geometry::squared_distance_to_rect(
                    (anchor.x, anchor.y),
                    (monitor.bounds.left, monitor.bounds.top),
                    (monitor.bounds.width(), monitor.bounds.height()),
                )
            })
        })
}

pub(super) fn resolve_surface_anchor<E>(
    valid_caret: Option<PixelPoint>,
    cursor_fallback: impl FnOnce() -> Result<PixelPoint, E>,
) -> Result<(PixelPoint, SurfaceAnchorSource), E> {
    if let Some(caret) = valid_caret {
        Ok((caret, SurfaceAnchorSource::Caret))
    } else {
        cursor_fallback().map(|cursor| (cursor, SurfaceAnchorSource::Cursor))
    }
}

pub(super) fn convert_caret_client_to_physical_screen<T: Copy, M, E>(
    client_point: T,
    client_to_screen: impl FnOnce(T) -> Result<T, E>,
    logical_to_physical: impl FnOnce(T) -> Result<(T, M), E>,
) -> Result<(T, T, M), E> {
    let logical_screen = client_to_screen(client_point)?;
    let (physical_screen, mode) = logical_to_physical(logical_screen)?;
    Ok((logical_screen, physical_screen, mode))
}

pub(super) fn surface_placement(
    cursor: PixelPoint,
    monitor: MonitorGeometry,
) -> Option<SurfacePlacement> {
    if !monitor.bounds.is_valid()
        || !monitor.work_area.is_valid()
        || !monitor.scale_factor.is_finite()
        || monitor.scale_factor <= 0.0
    {
        return None;
    }
    let requested_width = scaled_dimension(CLIPBOARD_SURFACE_WIDTH, monitor.scale_factor)?;
    let requested_height = scaled_dimension(CLIPBOARD_SURFACE_HEIGHT, monitor.scale_factor)?;
    let size = PixelSize {
        width: requested_width.min(monitor.work_area.width()),
        height: requested_height.min(monitor.work_area.height()),
    };
    if size.width == 0 || size.height == 0 {
        return None;
    }
    let gap = scaled_dimension(CLIPBOARD_SURFACE_CURSOR_GAP, monitor.scale_factor)?;
    let gap = i32::try_from(gap).ok()?;
    Some(SurfacePlacement {
        position: PixelPoint {
            x: place_axis(
                cursor.x,
                monitor.work_area.left,
                monitor.work_area.right,
                size.width,
                gap,
            )?,
            y: place_axis(
                cursor.y,
                monitor.work_area.top,
                monitor.work_area.bottom,
                size.height,
                gap,
            )?,
        },
        size,
    })
}

pub(super) fn preview_surface_placement(
    anchor: PixelRect,
    monitor: MonitorGeometry,
) -> Option<SurfacePlacement> {
    if !anchor.is_valid()
        || !monitor.bounds.is_valid()
        || !monitor.work_area.is_valid()
        || !monitor.scale_factor.is_finite()
        || monitor.scale_factor <= 0.0
    {
        return None;
    }
    let requested_width = scaled_dimension(CLIPBOARD_PREVIEW_SURFACE_WIDTH, monitor.scale_factor)?;
    let requested_height =
        scaled_dimension(CLIPBOARD_PREVIEW_SURFACE_HEIGHT, monitor.scale_factor)?;
    let right_start = anchor.right;
    let left_end = anchor.left;
    let right_space = axis_space(right_start, monitor.work_area.right);
    let left_space = axis_space(monitor.work_area.left, left_end);
    let (on_right, available_width) = if right_space >= requested_width {
        (true, right_space)
    } else if left_space >= requested_width {
        (false, left_space)
    } else if right_space >= left_space {
        (true, right_space)
    } else {
        (false, left_space)
    };
    let width = requested_width.min(available_width);
    let height = requested_height.min(monitor.work_area.height());
    if width == 0 || height == 0 {
        return None;
    }
    let width_i32 = i32::try_from(width).ok()?;
    let x = if on_right {
        right_start
    } else {
        left_end.checked_sub(width_i32)?
    };
    let maximum_y = i64::from(monitor.work_area.bottom).checked_sub(i64::from(height))?;
    let y = i64::from(anchor.top).clamp(i64::from(monitor.work_area.top), maximum_y);
    Some(SurfacePlacement {
        position: PixelPoint {
            x,
            y: i32::try_from(y).ok()?,
        },
        size: PixelSize { width, height },
    })
}

fn axis_space(start: i32, end: i32) -> u32 {
    u32::try_from(i64::from(end) - i64::from(start)).unwrap_or(0)
}

fn scaled_dimension(logical: f64, scale_factor: f64) -> Option<u32> {
    let physical = logical * scale_factor;
    if !physical.is_finite() || physical <= 0.0 || physical > f64::from(u32::MAX) {
        return None;
    }
    u32::try_from(physical.round() as u64).ok()
}

fn place_axis(cursor: i32, start: i32, end: i32, length: u32, gap: i32) -> Option<i32> {
    let start = i64::from(start);
    let end = i64::from(end);
    let cursor = i64::from(cursor);
    let length = i64::from(length);
    let gap = i64::from(gap);
    if end <= start || length <= 0 || length > end - start || gap < 0 {
        return None;
    }
    let after = cursor.checked_add(gap)?;
    if after >= start && after.checked_add(length)? <= end {
        return i32::try_from(after).ok();
    }
    let before = cursor.checked_sub(gap)?.checked_sub(length)?;
    if before >= start && before.checked_add(length)? <= end {
        return i32::try_from(before).ok();
    }
    i32::try_from(after.clamp(start, end - length)).ok()
}
