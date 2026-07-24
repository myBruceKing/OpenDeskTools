use super::model::{CapturedImage, PhysicalPoint, PhysicalRect};
use super::ScreenshotError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationTool {
    Rectangle,
    Arrow,
    Pen,
    Text,
    Mosaic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnnotationStyle {
    pub color: [u8; 3],
    pub thickness: u8,
    pub font_size: u8,
    pub mosaic_block: u8,
}

impl Default for AnnotationStyle {
    fn default() -> Self {
        Self {
            color: [238, 49, 49],
            thickness: 3,
            font_size: 18,
            mosaic_block: 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    pub tool: AnnotationTool,
    pub points: Vec<PhysicalPoint>,
    pub style: AnnotationStyle,
    pub text: Option<String>,
}

impl Annotation {
    pub fn new(tool: AnnotationTool, start: PhysicalPoint) -> Self {
        Self::with_style(tool, start, AnnotationStyle::default())
    }

    pub fn with_style(tool: AnnotationTool, start: PhysicalPoint, style: AnnotationStyle) -> Self {
        Self {
            tool,
            points: vec![start],
            style,
            text: None,
        }
    }

    pub fn text(start: PhysicalPoint, text: String, style: AnnotationStyle) -> Self {
        Self {
            tool: AnnotationTool::Text,
            points: vec![start],
            style,
            text: Some(text),
        }
    }

    pub fn update(&mut self, point: PhysicalPoint) {
        match self.tool {
            AnnotationTool::Pen => {
                if self.points.last().copied() != Some(point) {
                    self.points.push(point);
                }
            }
            AnnotationTool::Text => {}
            AnnotationTool::Rectangle | AnnotationTool::Arrow | AnnotationTool::Mosaic => {
                if self.points.len() == 1 {
                    self.points.push(point);
                } else {
                    self.points[1] = point;
                }
            }
        }
    }

    pub fn is_visible(&self) -> bool {
        match self.tool {
            AnnotationTool::Pen => self.points.len() >= 2,
            AnnotationTool::Text => self
                .text
                .as_deref()
                .is_some_and(|text| !text.trim().is_empty()),
            AnnotationTool::Rectangle | AnnotationTool::Arrow | AnnotationTool::Mosaic => {
                self.points.len() == 2 && self.points[0] != self.points[1]
            }
        }
    }
}

pub fn apply_annotations(
    image: &mut CapturedImage,
    selection: PhysicalRect,
    annotations: &[Annotation],
) -> Result<(), ScreenshotError> {
    for annotation in annotations {
        match annotation.tool {
            AnnotationTool::Rectangle => draw_rectangle(image, selection, annotation)?,
            AnnotationTool::Arrow => draw_arrow(image, selection, annotation)?,
            AnnotationTool::Pen => draw_pen(image, selection, annotation)?,
            AnnotationTool::Text => draw_text(image, selection, annotation)?,
            AnnotationTool::Mosaic => draw_mosaic(image, selection, annotation)?,
        }
    }
    Ok(())
}

fn draw_rectangle(
    image: &mut CapturedImage,
    selection: PhysicalRect,
    annotation: &Annotation,
) -> Result<(), ScreenshotError> {
    let Some((start, end)) = endpoints(annotation) else {
        return Ok(());
    };
    let left = start.x.min(end.x);
    let right = start.x.max(end.x);
    let top = start.y.min(end.y);
    let bottom = start.y.max(end.y);
    let color = annotation.style.color;
    for offset in thickness_offsets(annotation.style.thickness) {
        draw_line(
            image,
            selection,
            PhysicalPoint::new(left, top + offset),
            PhysicalPoint::new(right, top + offset),
            color,
            255,
        )?;
        draw_line(
            image,
            selection,
            PhysicalPoint::new(left, bottom + offset),
            PhysicalPoint::new(right, bottom + offset),
            color,
            255,
        )?;
        draw_line(
            image,
            selection,
            PhysicalPoint::new(left + offset, top),
            PhysicalPoint::new(left + offset, bottom),
            color,
            255,
        )?;
        draw_line(
            image,
            selection,
            PhysicalPoint::new(right + offset, top),
            PhysicalPoint::new(right + offset, bottom),
            color,
            255,
        )?;
    }
    Ok(())
}

fn draw_arrow(
    image: &mut CapturedImage,
    selection: PhysicalRect,
    annotation: &Annotation,
) -> Result<(), ScreenshotError> {
    let Some((start, end)) = endpoints(annotation) else {
        return Ok(());
    };
    let color = annotation.style.color;
    let thickness = i32::from(annotation.style.thickness.clamp(1, 12));
    draw_thick_line(image, selection, start, end, color, thickness, 255)?;
    let dx = f64::from(end.x - start.x);
    let dy = f64::from(end.y - start.y);
    let length = (dx * dx + dy * dy).sqrt();
    if length < 2.0 {
        return Ok(());
    }
    let head = length.mul_add(0.24, 0.0).clamp(10.0, 22.0);
    let angle = dy.atan2(dx);
    for wing in [angle + 2.55, angle - 2.55] {
        let wing_end = PhysicalPoint::new(
            end.x + (head * wing.cos()).round() as i32,
            end.y + (head * wing.sin()).round() as i32,
        );
        draw_thick_line(image, selection, end, wing_end, color, thickness, 255)?;
    }
    Ok(())
}

fn draw_pen(
    image: &mut CapturedImage,
    selection: PhysicalRect,
    annotation: &Annotation,
) -> Result<(), ScreenshotError> {
    for pair in annotation.points.windows(2) {
        draw_thick_line(
            image,
            selection,
            pair[0],
            pair[1],
            annotation.style.color,
            i32::from(annotation.style.thickness.clamp(1, 12)),
            255,
        )?;
    }
    Ok(())
}

#[cfg(windows)]
fn draw_text(
    image: &mut CapturedImage,
    selection: PhysicalRect,
    annotation: &Annotation,
) -> Result<(), ScreenshotError> {
    use std::mem::{size_of, zeroed};
    use std::ptr::{copy_nonoverlapping, null_mut};

    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW,
        SelectObject, SetBkMode, SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS,
        DT_LEFT, DT_NOPREFIX, DT_TOP, FF_DONTCARE, FW_NORMAL, HGDIOBJ, OUT_DEFAULT_PRECIS,
        TRANSPARENT,
    };

    let Some(anchor) = annotation.points.first().copied() else {
        return Ok(());
    };
    let Some(text) = annotation
        .text
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    else {
        return Ok(());
    };
    let width = i32::try_from(image.width).map_err(|_| ScreenshotError::ArithmeticOverflow)?;
    let height = i32::try_from(image.height).map_err(|_| ScreenshotError::ArithmeticOverflow)?;
    let byte_count = image
        .width
        .checked_mul(image.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or(ScreenshotError::ArithmeticOverflow)?;
    if image.rgba.len() != byte_count {
        return Err(ScreenshotError::InvalidFrame);
    }

    let mut bgra = vec![0u8; byte_count];
    for (source, destination) in image.rgba.chunks_exact(4).zip(bgra.chunks_exact_mut(4)) {
        destination.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
    }
    let mut info: BITMAPINFO = unsafe { zeroed() };
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width,
        biHeight: -height,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        biSizeImage: u32::try_from(byte_count).unwrap_or(u32::MAX),
        ..unsafe { zeroed() }
    };
    let device_context = unsafe { CreateCompatibleDC(null_mut()) };
    if device_context.is_null() {
        return Err(ScreenshotError::WindowsApi("CreateCompatibleDC"));
    }
    let mut pixels = null_mut();
    let bitmap = unsafe {
        CreateDIBSection(
            device_context,
            &info,
            DIB_RGB_COLORS,
            &mut pixels,
            null_mut(),
            0,
        )
    };
    if bitmap.is_null() || pixels.is_null() {
        unsafe {
            let _ = DeleteDC(device_context);
        }
        return Err(ScreenshotError::WindowsApi("CreateDIBSection"));
    }
    unsafe {
        copy_nonoverlapping(bgra.as_ptr(), pixels.cast::<u8>(), byte_count);
    }
    let previous_bitmap = unsafe { SelectObject(device_context, bitmap as HGDIOBJ) };
    let face: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
    let font = unsafe {
        CreateFontW(
            -i32::from(annotation.style.font_size.clamp(10, 72)),
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            face.as_ptr(),
        )
    };
    let previous_font =
        (!font.is_null()).then(|| unsafe { SelectObject(device_context, font as HGDIOBJ) });
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut bounds = windows_sys::Win32::Foundation::RECT {
        left: anchor.x.saturating_sub(selection.left).clamp(0, width - 1),
        top: anchor.y.saturating_sub(selection.top).clamp(0, height - 1),
        right: width,
        bottom: height,
    };
    unsafe {
        let _ = SetBkMode(device_context, TRANSPARENT as i32);
        let color = u32::from(annotation.style.color[0])
            | (u32::from(annotation.style.color[1]) << 8)
            | (u32::from(annotation.style.color[2]) << 16);
        let _ = SetTextColor(device_context, color);
        let _ = DrawTextW(
            device_context,
            wide.as_ptr(),
            wide.len() as i32,
            &mut bounds,
            DT_LEFT | DT_TOP | DT_NOPREFIX,
        );
        copy_nonoverlapping(pixels.cast::<u8>(), bgra.as_mut_ptr(), byte_count);
    }
    for (source, destination) in bgra.chunks_exact(4).zip(image.rgba.chunks_exact_mut(4)) {
        destination.copy_from_slice(&[source[2], source[1], source[0], 255]);
    }
    unsafe {
        if let Some(previous_font) = previous_font {
            if !previous_font.is_null() {
                let _ = SelectObject(device_context, previous_font);
            }
        }
        if !font.is_null() {
            let _ = DeleteObject(font as HGDIOBJ);
        }
        if !previous_bitmap.is_null() {
            let _ = SelectObject(device_context, previous_bitmap);
        }
        let _ = DeleteObject(bitmap as HGDIOBJ);
        let _ = DeleteDC(device_context);
    }
    Ok(())
}

#[cfg(not(windows))]
fn draw_text(
    _image: &mut CapturedImage,
    _selection: PhysicalRect,
    _annotation: &Annotation,
) -> Result<(), ScreenshotError> {
    Err(ScreenshotError::UnsupportedPlatform)
}

fn draw_mosaic(
    image: &mut CapturedImage,
    selection: PhysicalRect,
    annotation: &Annotation,
) -> Result<(), ScreenshotError> {
    let Some((start, end)) = endpoints(annotation) else {
        return Ok(());
    };
    let left = start.x.min(end.x).max(selection.left);
    let right = start.x.max(end.x).min(selection.right - 1);
    let top = start.y.min(end.y).max(selection.top);
    let bottom = start.y.max(end.y).min(selection.bottom - 1);
    if left > right || top > bottom {
        return Ok(());
    }
    let block = i32::from(annotation.style.mosaic_block.clamp(4, 32));
    let mut y = top;
    while y <= bottom {
        let block_bottom = (y + block).min(bottom + 1);
        let mut x = left;
        while x <= right {
            let block_right = (x + block).min(right + 1);
            let color = average_block(image, selection, x, y, block_right, block_bottom)?;
            for pixel_y in y..block_bottom {
                for pixel_x in x..block_right {
                    blend_pixel(image, selection, pixel_x, pixel_y, color, 255)?;
                }
            }
            x = block_right;
        }
        y = block_bottom;
    }
    Ok(())
}

fn endpoints(annotation: &Annotation) -> Option<(PhysicalPoint, PhysicalPoint)> {
    (annotation.points.len() >= 2)
        .then(|| (annotation.points[0], *annotation.points.last().unwrap()))
}

fn thickness_offsets(thickness: u8) -> std::ops::RangeInclusive<i32> {
    let thickness = i32::from(thickness.clamp(1, 12));
    let before = thickness / 2;
    -before..=(thickness - before - 1)
}

fn draw_thick_line(
    image: &mut CapturedImage,
    selection: PhysicalRect,
    start: PhysicalPoint,
    end: PhysicalPoint,
    color: [u8; 3],
    thickness: i32,
    alpha: u8,
) -> Result<(), ScreenshotError> {
    let offsets = thickness_offsets(thickness.clamp(1, 12) as u8);
    for y in offsets.clone() {
        for x in offsets.clone() {
            draw_line(
                image,
                selection,
                PhysicalPoint::new(start.x + x, start.y + y),
                PhysicalPoint::new(end.x + x, end.y + y),
                color,
                alpha,
            )?;
        }
    }
    Ok(())
}

fn draw_line(
    image: &mut CapturedImage,
    selection: PhysicalRect,
    start: PhysicalPoint,
    end: PhysicalPoint,
    color: [u8; 3],
    alpha: u8,
) -> Result<(), ScreenshotError> {
    let mut x = start.x;
    let mut y = start.y;
    let dx = (end.x - start.x).abs();
    let step_x = if start.x < end.x { 1 } else { -1 };
    let dy = -(end.y - start.y).abs();
    let step_y = if start.y < end.y { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        blend_pixel(image, selection, x, y, color, alpha)?;
        if x == end.x && y == end.y {
            break;
        }
        let twice = error.saturating_mul(2);
        if twice >= dy {
            error += dy;
            x += step_x;
        }
        if twice <= dx {
            error += dx;
            y += step_y;
        }
    }
    Ok(())
}

fn average_block(
    image: &CapturedImage,
    selection: PhysicalRect,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
) -> Result<[u8; 3], ScreenshotError> {
    let mut total = [0u64; 3];
    let mut count = 0u64;
    for y in top..bottom {
        for x in left..right {
            let offset = pixel_offset(image, selection, x, y)?;
            for (channel, total_channel) in total.iter_mut().enumerate() {
                *total_channel += u64::from(image.rgba[offset + channel]);
            }
            count += 1;
        }
    }
    if count == 0 {
        return Ok([0, 0, 0]);
    }
    Ok([
        (total[0] / count) as u8,
        (total[1] / count) as u8,
        (total[2] / count) as u8,
    ])
}

fn blend_pixel(
    image: &mut CapturedImage,
    selection: PhysicalRect,
    x: i32,
    y: i32,
    color: [u8; 3],
    alpha: u8,
) -> Result<(), ScreenshotError> {
    if x < selection.left || x >= selection.right || y < selection.top || y >= selection.bottom {
        return Ok(());
    }
    let offset = pixel_offset(image, selection, x, y)?;
    let alpha = u16::from(alpha);
    let inverse = 255 - alpha;
    for (channel, source) in color.iter().enumerate() {
        image.rgba[offset + channel] =
            ((u16::from(image.rgba[offset + channel]) * inverse + u16::from(*source) * alpha + 127)
                / 255) as u8;
    }
    image.rgba[offset + 3] = 255;
    Ok(())
}

fn pixel_offset(
    image: &CapturedImage,
    selection: PhysicalRect,
    x: i32,
    y: i32,
) -> Result<usize, ScreenshotError> {
    let local_x =
        usize::try_from(x - selection.left).map_err(|_| ScreenshotError::ArithmeticOverflow)?;
    let local_y =
        usize::try_from(y - selection.top).map_err(|_| ScreenshotError::ArithmeticOverflow)?;
    let width = usize::try_from(image.width).map_err(|_| ScreenshotError::ArithmeticOverflow)?;
    local_y
        .checked_mul(width)
        .and_then(|row| row.checked_add(local_x))
        .and_then(|pixel| pixel.checked_mul(4))
        .filter(|offset| offset.saturating_add(4) <= image.rgba.len())
        .ok_or(ScreenshotError::ArithmeticOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> CapturedImage {
        CapturedImage::new(20, 20, vec![255; 20 * 20 * 4]).unwrap()
    }

    #[test]
    fn rectangle_is_flattened_into_selected_pixels() {
        let selection = PhysicalRect::new(100, 200, 120, 220).unwrap();
        let mut image = image();
        let mut annotation =
            Annotation::new(AnnotationTool::Rectangle, PhysicalPoint::new(103, 204));
        annotation.update(PhysicalPoint::new(110, 212));
        apply_annotations(&mut image, selection, &[annotation]).unwrap();
        let offset = ((4 * 20 + 3) * 4) as usize;
        assert_eq!(
            &image.rgba[offset..offset + 3],
            &AnnotationStyle::default().color
        );
        assert_eq!(&image.rgba[0..3], &[255, 255, 255]);
    }

    #[test]
    fn mosaic_replaces_a_block_with_its_average_color() {
        let selection = PhysicalRect::new(0, 0, 20, 20).unwrap();
        let mut image = image();
        image.rgba[0..4].copy_from_slice(&[0, 0, 0, 255]);
        let mut annotation = Annotation::new(AnnotationTool::Mosaic, PhysicalPoint::new(0, 0));
        annotation.update(PhysicalPoint::new(9, 9));
        apply_annotations(&mut image, selection, &[annotation]).unwrap();
        assert_eq!(&image.rgba[0..3], &image.rgba[4..7]);
        assert_ne!(&image.rgba[0..3], &[0, 0, 0]);
    }

    #[test]
    fn annotation_keeps_the_selected_color_and_thickness_for_flattening() {
        let selection = PhysicalRect::new(0, 0, 20, 20).unwrap();
        let mut image = image();
        let style = AnnotationStyle {
            color: [36, 112, 224],
            thickness: 8,
            font_size: 18,
            mosaic_block: 12,
        };
        let mut annotation =
            Annotation::with_style(AnnotationTool::Pen, PhysicalPoint::new(2, 10), style);
        annotation.update(PhysicalPoint::new(17, 10));
        apply_annotations(&mut image, selection, &[annotation]).unwrap();

        let center = ((10 * 20 + 10) * 4) as usize;
        let thick_edge = ((13 * 20 + 10) * 4) as usize;
        assert_eq!(&image.rgba[center..center + 3], &style.color);
        assert_eq!(&image.rgba[thick_edge..thick_edge + 3], &style.color);
    }

    #[test]
    fn text_annotation_keeps_content_and_is_flattened_into_pixels() {
        let selection = PhysicalRect::new(0, 0, 160, 60).unwrap();
        let mut image =
            CapturedImage::new(160, 60, vec![255; 160usize * 60usize * 4usize]).unwrap();
        let annotation = Annotation::text(
            PhysicalPoint::new(8, 8),
            "OpenDeskTools".to_owned(),
            AnnotationStyle {
                color: [36, 112, 224],
                font_size: 24,
                ..AnnotationStyle::default()
            },
        );
        assert!(annotation.is_visible());

        apply_annotations(&mut image, selection, &[annotation]).unwrap();

        assert!(image
            .rgba
            .chunks_exact(4)
            .any(|pixel| pixel[..3] != [255, 255, 255]));
    }
}
