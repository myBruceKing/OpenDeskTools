pub fn point_in_rect(point: (i32, i32), origin: (i32, i32), size: (u32, u32)) -> bool {
    let right = i64::from(origin.0) + i64::from(size.0);
    let bottom = i64::from(origin.1) + i64::from(size.1);
    i64::from(point.0) >= i64::from(origin.0)
        && i64::from(point.0) < right
        && i64::from(point.1) >= i64::from(origin.1)
        && i64::from(point.1) < bottom
}

pub fn squared_distance_to_rect(point: (i32, i32), origin: (i32, i32), size: (u32, u32)) -> i128 {
    let right = i64::from(origin.0) + i64::from(size.0);
    let bottom = i64::from(origin.1) + i64::from(size.1);
    let x = axis_distance(i64::from(point.0), i64::from(origin.0), right);
    let y = axis_distance(i64::from(point.1), i64::from(origin.1), bottom);
    i128::from(x) * i128::from(x) + i128::from(y) * i128::from(y)
}

pub fn fit_centered_surface_to_work_area(
    center: (i32, i32),
    surface_size: (u32, u32),
    work_origin: (i32, i32),
    work_size: (u32, u32),
) -> Option<(i32, i32)> {
    if surface_size.0 == 0
        || surface_size.1 == 0
        || surface_size.0 > work_size.0
        || surface_size.1 > work_size.1
    {
        return None;
    }
    let work_left = i64::from(work_origin.0);
    let work_top = i64::from(work_origin.1);
    let maximum_x = work_left + i64::from(work_size.0 - surface_size.0);
    let maximum_y = work_top + i64::from(work_size.1 - surface_size.1);
    let requested_x = i64::from(center.0) - i64::from(surface_size.0 / 2);
    let requested_y = i64::from(center.1) - i64::from(surface_size.1 / 2);
    Some((
        i32::try_from(requested_x.clamp(work_left, maximum_x)).ok()?,
        i32::try_from(requested_y.clamp(work_top, maximum_y)).ok()?,
    ))
}

pub fn top_right_position(
    work_origin: (i32, i32),
    work_size: (u32, u32),
    surface_size: (i32, i32),
    gap: i32,
) -> Option<(i32, i32)> {
    if surface_size.0 <= 0
        || surface_size.1 <= 0
        || gap < 0
        || i64::from(surface_size.0) + i64::from(gap) > i64::from(work_size.0)
        || i64::from(surface_size.1) + i64::from(gap) > i64::from(work_size.1)
    {
        return None;
    }
    let x = i64::from(work_origin.0) + i64::from(work_size.0)
        - i64::from(surface_size.0)
        - i64::from(gap);
    let y = i64::from(work_origin.1) + i64::from(gap);
    Some((i32::try_from(x).ok()?, i32::try_from(y).ok()?))
}

fn axis_distance(point: i64, start: i64, end: i64) -> i64 {
    if point < start {
        start - point
    } else if point >= end {
        point - end.saturating_sub(1)
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_helpers_support_negative_monitor_coordinates() {
        let origin = (-1920, -40);
        let size = (1920, 1080);

        assert!(point_in_rect((-1, 1039), origin, size));
        assert!(!point_in_rect((0, 1040), origin, size));
        assert_eq!(squared_distance_to_rect((-1921, -41), origin, size), 2);
    }

    #[test]
    fn centered_surface_is_clamped_inside_the_work_area() {
        assert_eq!(
            fit_centered_surface_to_work_area((1910, 1070), (320, 320), (0, 0), (1920, 1040)),
            Some((1600, 720))
        );
        assert_eq!(
            fit_centered_surface_to_work_area((-1915, -5), (388, 140), (-1920, -40), (1920, 1080)),
            Some((-1920, -40))
        );
        assert_eq!(
            fit_centered_surface_to_work_area((0, 0), (700, 700), (0, 0), (640, 480)),
            None
        );
    }

    #[test]
    fn top_right_surface_respects_work_area_origin_and_gap() {
        assert_eq!(
            top_right_position((0, 0), (1920, 1040), (390, 88), 16),
            Some((1514, 16))
        );
        assert_eq!(
            top_right_position((-1920, -40), (1920, 1080), (390, 88), 16),
            Some((-406, -24))
        );
    }
}
