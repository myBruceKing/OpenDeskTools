use super::model::{PhysicalPoint, PhysicalRect};
use super::ScreenshotError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionOutcome {
    Cancelled,
    Confirmed(PhysicalRect),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionHandle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionInteraction {
    Create {
        anchor: PhysicalPoint,
    },
    Move {
        pointer: PhysicalPoint,
        original: PhysicalRect,
    },
    Resize {
        pointer: PhysicalPoint,
        original: PhysicalRect,
        handle: SelectionHandle,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionState {
    bounds: PhysicalRect,
    interaction: Option<SelectionInteraction>,
    selection: Option<PhysicalRect>,
    outcome: Option<SelectionOutcome>,
}

impl SelectionState {
    pub fn new(bounds: PhysicalRect) -> Self {
        Self {
            bounds,
            interaction: None,
            selection: None,
            outcome: None,
        }
    }

    pub fn begin(&mut self, point: PhysicalPoint) {
        let point = self.clamp_point(point);
        self.interaction = Some(SelectionInteraction::Create { anchor: point });
        self.selection = rect_from_points(point, point, self.bounds);
        self.outcome = None;
    }

    pub fn set_selection(&mut self, selection: PhysicalRect) -> bool {
        let Some(selection) = selection.intersection(self.bounds) else {
            return false;
        };
        self.interaction = None;
        self.selection = Some(selection);
        self.outcome = None;
        true
    }

    pub fn begin_move(&mut self, point: PhysicalPoint) -> bool {
        let Some(original) = self.selection else {
            return false;
        };
        let point = self.clamp_point(point);
        if !contains(original, point) {
            return false;
        }
        self.interaction = Some(SelectionInteraction::Move {
            pointer: point,
            original,
        });
        self.outcome = None;
        true
    }

    pub fn begin_resize(&mut self, point: PhysicalPoint, handle: SelectionHandle) -> bool {
        let Some(original) = self.selection else {
            return false;
        };
        self.interaction = Some(SelectionInteraction::Resize {
            pointer: self.clamp_point(point),
            original,
            handle,
        });
        self.outcome = None;
        true
    }

    /// Starts a new drag unless the pointer is already inside the completed
    /// selection. Preserving the existing rectangle lets a double-click
    /// confirm it without the first click collapsing it to a 1×1 capture.
    pub fn begin_or_preserve(&mut self, point: PhysicalPoint) -> bool {
        let point = self.clamp_point(point);
        if !self.dragging()
            && self
                .selection
                .is_some_and(|selection| contains(selection, point))
        {
            self.interaction = None;
            self.outcome = None;
            return false;
        }
        self.begin(point);
        true
    }

    pub fn update(&mut self, point: PhysicalPoint) {
        let Some(interaction) = self.interaction else {
            return;
        };
        let point = self.clamp_point(point);
        self.selection = match interaction {
            SelectionInteraction::Create { anchor } => rect_from_points(anchor, point, self.bounds),
            SelectionInteraction::Move { pointer, original } => {
                Some(move_rect(original, pointer, point, self.bounds))
            }
            SelectionInteraction::Resize {
                pointer,
                original,
                handle,
            } => resize_rect(original, pointer, point, handle, self.bounds),
        };
    }

    pub fn finish(&mut self, point: PhysicalPoint) {
        self.update(point);
        self.interaction = None;
    }

    pub fn nudge(&mut self, delta_x: i32, delta_y: i32) -> bool {
        let Some(selection) = self.selection else {
            return false;
        };
        if self.dragging() {
            return false;
        }
        let width = selection.right - selection.left;
        let height = selection.bottom - selection.top;
        let left = selection
            .left
            .saturating_add(delta_x)
            .clamp(self.bounds.left, self.bounds.right - width);
        let top = selection
            .top
            .saturating_add(delta_y)
            .clamp(self.bounds.top, self.bounds.bottom - height);
        self.selection = PhysicalRect::new(left, top, left + width, top + height).ok();
        self.outcome = None;
        true
    }

    pub fn resize(&mut self, delta_width: i32, delta_height: i32) -> bool {
        let Some(selection) = self.selection else {
            return false;
        };
        if self.dragging() {
            return false;
        }
        let right = selection
            .right
            .saturating_add(delta_width)
            .clamp(selection.left + 1, self.bounds.right);
        let bottom = selection
            .bottom
            .saturating_add(delta_height)
            .clamp(selection.top + 1, self.bounds.bottom);
        self.selection = PhysicalRect::new(selection.left, selection.top, right, bottom).ok();
        self.outcome = None;
        true
    }

    pub fn cancel(&mut self) {
        self.interaction = None;
        self.outcome = Some(SelectionOutcome::Cancelled);
    }

    pub fn confirm(&mut self) -> bool {
        let Some(selection) = self.selection else {
            return false;
        };
        self.interaction = None;
        self.outcome = Some(SelectionOutcome::Confirmed(selection));
        true
    }

    pub fn selection(&self) -> Option<PhysicalRect> {
        self.selection
    }

    pub fn outcome(&self) -> Option<SelectionOutcome> {
        self.outcome
    }

    pub fn dragging(&self) -> bool {
        self.interaction.is_some()
    }

    fn clamp_point(&self, point: PhysicalPoint) -> PhysicalPoint {
        PhysicalPoint {
            x: point.x.clamp(self.bounds.left, self.bounds.right - 1),
            y: point.y.clamp(self.bounds.top, self.bounds.bottom - 1),
        }
    }
}

fn move_rect(
    original: PhysicalRect,
    pointer: PhysicalPoint,
    current: PhysicalPoint,
    bounds: PhysicalRect,
) -> PhysicalRect {
    let width = original.right - original.left;
    let height = original.bottom - original.top;
    let delta_x = current.x.saturating_sub(pointer.x);
    let delta_y = current.y.saturating_sub(pointer.y);
    let left = original
        .left
        .saturating_add(delta_x)
        .clamp(bounds.left, bounds.right - width);
    let top = original
        .top
        .saturating_add(delta_y)
        .clamp(bounds.top, bounds.bottom - height);
    PhysicalRect {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

fn resize_rect(
    original: PhysicalRect,
    pointer: PhysicalPoint,
    current: PhysicalPoint,
    handle: SelectionHandle,
    bounds: PhysicalRect,
) -> Option<PhysicalRect> {
    let delta_x = current.x.saturating_sub(pointer.x);
    let delta_y = current.y.saturating_sub(pointer.y);
    let moves_left = matches!(
        handle,
        SelectionHandle::TopLeft | SelectionHandle::Left | SelectionHandle::BottomLeft
    );
    let moves_right = matches!(
        handle,
        SelectionHandle::TopRight | SelectionHandle::Right | SelectionHandle::BottomRight
    );
    let moves_top = matches!(
        handle,
        SelectionHandle::TopLeft | SelectionHandle::Top | SelectionHandle::TopRight
    );
    let moves_bottom = matches!(
        handle,
        SelectionHandle::BottomLeft | SelectionHandle::Bottom | SelectionHandle::BottomRight
    );
    let left = if moves_left {
        original
            .left
            .saturating_add(delta_x)
            .clamp(bounds.left, original.right - 1)
    } else {
        original.left
    };
    let right = if moves_right {
        original
            .right
            .saturating_add(delta_x)
            .clamp(original.left + 1, bounds.right)
    } else {
        original.right
    };
    let top = if moves_top {
        original
            .top
            .saturating_add(delta_y)
            .clamp(bounds.top, original.bottom - 1)
    } else {
        original.top
    };
    let bottom = if moves_bottom {
        original
            .bottom
            .saturating_add(delta_y)
            .clamp(original.top + 1, bounds.bottom)
    } else {
        original.bottom
    };
    PhysicalRect::new(left, top, right, bottom).ok()
}

fn rect_from_points(
    first: PhysicalPoint,
    second: PhysicalPoint,
    bounds: PhysicalRect,
) -> Option<PhysicalRect> {
    let left = first.x.min(second.x).max(bounds.left);
    let top = first.y.min(second.y).max(bounds.top);
    let right = first.x.max(second.x).saturating_add(1).min(bounds.right);
    let bottom = first.y.max(second.y).saturating_add(1).min(bounds.bottom);
    PhysicalRect::new(left, top, right, bottom).ok()
}

fn contains(rect: PhysicalRect, point: PhysicalPoint) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

pub fn selection_size(selection: PhysicalRect) -> Result<(u32, u32), ScreenshotError> {
    Ok((
        selection.width().ok_or(ScreenshotError::InvalidRect)?,
        selection.height().ok_or(ScreenshotError::InvalidRect)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> PhysicalRect {
        PhysicalRect::new(-100, -50, 200, 150).unwrap()
    }

    #[test]
    fn drag_normalizes_reverse_direction_and_uses_exclusive_edges() {
        let mut state = SelectionState::new(bounds());
        state.begin(PhysicalPoint::new(20, 30));
        state.finish(PhysicalPoint::new(10, 5));
        assert_eq!(
            state.selection(),
            Some(PhysicalRect::new(10, 5, 21, 31).unwrap())
        );
        assert!(!state.dragging());
    }

    #[test]
    fn drag_clamps_to_negative_virtual_desktop_bounds() {
        let mut state = SelectionState::new(bounds());
        state.begin(PhysicalPoint::new(-500, -500));
        state.finish(PhysicalPoint::new(500, 500));
        assert_eq!(state.selection(), Some(bounds()));
    }

    #[test]
    fn confirm_requires_a_selection_and_cancel_wins_without_one() {
        let mut state = SelectionState::new(bounds());
        assert!(!state.confirm());
        state.cancel();
        assert_eq!(state.outcome(), Some(SelectionOutcome::Cancelled));

        state.begin(PhysicalPoint::new(0, 0));
        state.finish(PhysicalPoint::new(9, 4));
        assert!(state.confirm());
        assert_eq!(
            state.outcome(),
            Some(SelectionOutcome::Confirmed(
                PhysicalRect::new(0, 0, 10, 5).unwrap()
            ))
        );
    }

    #[test]
    fn arrow_nudge_preserves_size_and_clamps_to_virtual_bounds() {
        let mut state = SelectionState::new(bounds());
        state.begin(PhysicalPoint::new(-90, -40));
        state.finish(PhysicalPoint::new(-81, -31));

        assert!(state.nudge(-50, -50));
        assert_eq!(
            state.selection(),
            Some(PhysicalRect::new(-100, -50, -90, -40).unwrap())
        );
        assert!(state.nudge(1_000, 1_000));
        assert_eq!(
            state.selection(),
            Some(PhysicalRect::new(190, 140, 200, 150).unwrap())
        );
    }

    #[test]
    fn shift_arrow_resize_uses_bottom_right_edge_and_keeps_one_pixel_minimum() {
        let mut state = SelectionState::new(bounds());
        state.begin(PhysicalPoint::new(10, 20));
        state.finish(PhysicalPoint::new(19, 29));

        assert!(state.resize(5, -4));
        assert_eq!(
            state.selection(),
            Some(PhysicalRect::new(10, 20, 25, 26).unwrap())
        );
        assert!(state.resize(-1_000, -1_000));
        assert_eq!(
            state.selection(),
            Some(PhysicalRect::new(10, 20, 11, 21).unwrap())
        );
        assert!(state.resize(1_000, 1_000));
        assert_eq!(
            state.selection(),
            Some(PhysicalRect::new(10, 20, 200, 150).unwrap())
        );
    }

    #[test]
    fn click_inside_completed_selection_preserves_it_for_double_click_confirmation() {
        let mut state = SelectionState::new(bounds());
        state.begin(PhysicalPoint::new(10, 20));
        state.finish(PhysicalPoint::new(109, 69));
        let expected = PhysicalRect::new(10, 20, 110, 70).unwrap();

        assert!(!state.begin_or_preserve(PhysicalPoint::new(40, 40)));
        assert_eq!(state.selection(), Some(expected));
        assert!(!state.dragging());
        assert!(state.confirm());
        assert_eq!(state.outcome(), Some(SelectionOutcome::Confirmed(expected)));
    }

    #[test]
    fn click_outside_completed_selection_starts_a_new_selection() {
        let mut state = SelectionState::new(bounds());
        state.begin(PhysicalPoint::new(10, 20));
        state.finish(PhysicalPoint::new(109, 69));

        assert!(state.begin_or_preserve(PhysicalPoint::new(-20, -10)));
        assert_eq!(
            state.selection(),
            Some(PhysicalRect::new(-20, -10, -19, -9).unwrap())
        );
        assert!(state.dragging());
    }

    #[test]
    fn completed_selection_can_move_without_changing_its_size() {
        let mut state = SelectionState::new(bounds());
        state.begin(PhysicalPoint::new(10, 20));
        state.finish(PhysicalPoint::new(109, 69));

        assert!(state.begin_move(PhysicalPoint::new(40, 40)));
        state.finish(PhysicalPoint::new(60, 55));
        assert_eq!(
            state.selection(),
            Some(PhysicalRect::new(30, 35, 130, 85).unwrap())
        );
    }

    #[test]
    fn every_resize_handle_changes_only_its_owned_edges() {
        let mut state = SelectionState::new(bounds());
        state.begin(PhysicalPoint::new(10, 20));
        state.finish(PhysicalPoint::new(109, 69));

        assert!(state.begin_resize(PhysicalPoint::new(10, 20), SelectionHandle::TopLeft));
        state.finish(PhysicalPoint::new(0, 5));
        assert_eq!(
            state.selection(),
            Some(PhysicalRect::new(0, 5, 110, 70).unwrap())
        );

        assert!(state.begin_resize(PhysicalPoint::new(109, 69), SelectionHandle::BottomRight));
        state.finish(PhysicalPoint::new(150, 100));
        assert_eq!(
            state.selection(),
            Some(PhysicalRect::new(0, 5, 151, 101).unwrap())
        );
    }

    #[test]
    fn smart_candidate_can_be_fixed_and_is_clipped_to_the_virtual_desktop() {
        let mut state = SelectionState::new(bounds());
        assert!(state.set_selection(PhysicalRect::new(-200, -100, 50, 80).unwrap()));
        assert_eq!(
            state.selection(),
            Some(PhysicalRect::new(-100, -50, 50, 80).unwrap())
        );
        assert!(!state.dragging());
    }
}
