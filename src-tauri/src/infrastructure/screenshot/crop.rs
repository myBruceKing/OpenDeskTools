use super::model::{CapturedImage, PhysicalRect, VirtualDesktopSnapshot};
use super::{ScreenshotError, MAX_CAPTURED_IMAGE_BYTES};

pub fn crop_snapshot(
    snapshot: &VirtualDesktopSnapshot,
    selection: PhysicalRect,
) -> Result<CapturedImage, ScreenshotError> {
    if snapshot.virtual_bounds.intersection(selection) != Some(selection) {
        return Err(ScreenshotError::SelectionOutsideDesktop);
    }
    let width = selection.width().ok_or(ScreenshotError::InvalidRect)?;
    let height = selection.height().ok_or(ScreenshotError::InvalidRect)?;
    let output_len = rgba_len(width, height)?;
    if output_len > MAX_CAPTURED_IMAGE_BYTES {
        return Err(ScreenshotError::MemoryLimit);
    }

    // A rectangular selection may cross an unused gap between monitors.
    // Keep those pixels opaque black so clipboard/file consumers receive an
    // ordinary desktop bitmap rather than unexpected transparency.
    let mut rgba = vec![0u8; output_len];
    for alpha in rgba.iter_mut().skip(3).step_by(4) {
        *alpha = 255;
    }

    let output_width = usize::try_from(width).map_err(|_| ScreenshotError::ArithmeticOverflow)?;
    let mut copied_any = false;
    for frame in &snapshot.frames {
        frame.validate()?;
        let Some(intersection) = frame.monitor.physical_bounds.intersection(selection) else {
            continue;
        };
        copied_any = true;
        copy_intersection(frame, selection, intersection, output_width, &mut rgba)?;
    }
    if !copied_any {
        return Err(ScreenshotError::SelectionOutsideDesktop);
    }
    CapturedImage::new(width, height, rgba)
}

fn rgba_len(width: u32, height: u32) -> Result<usize, ScreenshotError> {
    usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(usize::try_from(height).ok()?))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(ScreenshotError::ArithmeticOverflow)
}

fn copy_intersection(
    frame: &super::model::MonitorFrame,
    selection: PhysicalRect,
    intersection: PhysicalRect,
    output_width: usize,
    output: &mut [u8],
) -> Result<(), ScreenshotError> {
    let copy_width = usize::try_from(
        intersection
            .width()
            .ok_or(ScreenshotError::ArithmeticOverflow)?,
    )
    .map_err(|_| ScreenshotError::ArithmeticOverflow)?;
    let copy_height = usize::try_from(
        intersection
            .height()
            .ok_or(ScreenshotError::ArithmeticOverflow)?,
    )
    .map_err(|_| ScreenshotError::ArithmeticOverflow)?;
    let source_x = coordinate_offset(intersection.left, frame.monitor.physical_bounds.left)?;
    let source_y = coordinate_offset(intersection.top, frame.monitor.physical_bounds.top)?;
    let destination_x = coordinate_offset(intersection.left, selection.left)?;
    let destination_y = coordinate_offset(intersection.top, selection.top)?;

    for row in 0..copy_height {
        let source_row = source_y
            .checked_add(row)
            .and_then(|value| value.checked_mul(frame.stride))
            .ok_or(ScreenshotError::ArithmeticOverflow)?;
        let source_start = source_row
            .checked_add(
                source_x
                    .checked_mul(4)
                    .ok_or(ScreenshotError::ArithmeticOverflow)?,
            )
            .ok_or(ScreenshotError::ArithmeticOverflow)?;
        let destination_row = destination_y
            .checked_add(row)
            .and_then(|value| value.checked_mul(output_width))
            .ok_or(ScreenshotError::ArithmeticOverflow)?;
        let destination_start = destination_row
            .checked_add(destination_x)
            .and_then(|value| value.checked_mul(4))
            .ok_or(ScreenshotError::ArithmeticOverflow)?;
        for column in 0..copy_width {
            let source = source_start
                .checked_add(
                    column
                        .checked_mul(4)
                        .ok_or(ScreenshotError::ArithmeticOverflow)?,
                )
                .ok_or(ScreenshotError::ArithmeticOverflow)?;
            let destination = destination_start
                .checked_add(
                    column
                        .checked_mul(4)
                        .ok_or(ScreenshotError::ArithmeticOverflow)?,
                )
                .ok_or(ScreenshotError::ArithmeticOverflow)?;
            let source_pixel = frame
                .bgra
                .get(source..source + 4)
                .ok_or(ScreenshotError::InvalidFrame)?;
            let destination_pixel = output
                .get_mut(destination..destination + 4)
                .ok_or(ScreenshotError::InvalidFrame)?;
            destination_pixel.copy_from_slice(&[
                source_pixel[2],
                source_pixel[1],
                source_pixel[0],
                source_pixel[3],
            ]);
        }
    }
    Ok(())
}

fn coordinate_offset(value: i32, origin: i32) -> Result<usize, ScreenshotError> {
    let offset = i64::from(value)
        .checked_sub(i64::from(origin))
        .ok_or(ScreenshotError::ArithmeticOverflow)?;
    usize::try_from(offset).map_err(|_| ScreenshotError::ArithmeticOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::screenshot::model::{
        BackendReport, CaptureBackendName, DisplayRotation, MonitorDescriptor, MonitorFrame,
    };

    fn frame(id: &str, bounds: PhysicalRect, pixels: &[[u8; 4]]) -> MonitorFrame {
        let width = bounds.width().unwrap();
        let height = bounds.height().unwrap();
        assert_eq!(pixels.len(), width as usize * height as usize);
        MonitorFrame {
            monitor: MonitorDescriptor {
                id: id.to_owned(),
                physical_bounds: bounds,
                work_bounds: bounds,
                dpi_x: 96,
                dpi_y: 96,
                rotation: DisplayRotation::Identity,
                is_primary: false,
            },
            width,
            height,
            stride: width as usize * 4,
            bgra: pixels.iter().flatten().copied().collect(),
        }
    }

    #[test]
    fn crop_converts_bgra_to_rgba() {
        let bounds = PhysicalRect::new(0, 0, 2, 1).unwrap();
        let snapshot = VirtualDesktopSnapshot::new(
            1,
            10,
            vec![frame(
                "display",
                bounds,
                &[[10, 20, 30, 255], [40, 50, 60, 128]],
            )],
            vec![BackendReport {
                monitor_id: "display".to_owned(),
                backend: CaptureBackendName::Fake,
                detail: "fixture".to_owned(),
            }],
        )
        .unwrap();
        let image = crop_snapshot(&snapshot, bounds).unwrap();
        assert_eq!(image.rgba, vec![30, 20, 10, 255, 60, 50, 40, 128]);
    }

    #[test]
    fn crop_composes_frames_across_negative_monitor_coordinates() {
        let left = PhysicalRect::new(-1, 0, 0, 1).unwrap();
        let right = PhysicalRect::new(0, 0, 1, 1).unwrap();
        let snapshot = VirtualDesktopSnapshot::new(
            2,
            20,
            vec![
                frame("left", left, &[[0, 0, 255, 255]]),
                frame("right", right, &[[255, 0, 0, 255]]),
            ],
            Vec::new(),
        )
        .unwrap();
        let image = crop_snapshot(&snapshot, PhysicalRect::new(-1, 0, 1, 1).unwrap()).unwrap();
        assert_eq!(image.rgba, vec![255, 0, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn crop_keeps_virtual_desktop_gaps_opaque_black() {
        let left = PhysicalRect::new(0, 0, 1, 1).unwrap();
        let right = PhysicalRect::new(2, 0, 3, 1).unwrap();
        let snapshot = VirtualDesktopSnapshot::new(
            3,
            30,
            vec![
                frame("left", left, &[[0, 255, 0, 255]]),
                frame("right", right, &[[0, 0, 255, 255]]),
            ],
            Vec::new(),
        )
        .unwrap();
        let image = crop_snapshot(&snapshot, PhysicalRect::new(0, 0, 3, 1).unwrap()).unwrap();
        assert_eq!(
            image.rgba,
            vec![0, 255, 0, 255, 0, 0, 0, 255, 255, 0, 0, 255]
        );
    }

    #[test]
    fn crop_rejects_selection_outside_virtual_bounds() {
        let bounds = PhysicalRect::new(0, 0, 1, 1).unwrap();
        let snapshot = VirtualDesktopSnapshot::new(
            4,
            40,
            vec![frame("display", bounds, &[[0, 0, 0, 255]])],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(
            crop_snapshot(&snapshot, PhysicalRect::new(-1, 0, 1, 1).unwrap()),
            Err(ScreenshotError::SelectionOutsideDesktop)
        );
    }
}
