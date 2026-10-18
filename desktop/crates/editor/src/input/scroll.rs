use gpui::Pixels;
use std::cell::Cell;

#[derive(Default)]
pub struct EditorScrollState {
    offset: Cell<Pixels>,
    content_height: Cell<Pixels>,
    viewport_height: Cell<Pixels>,
}

impl EditorScrollState {
    pub fn offset(&self) -> Pixels {
        self.offset.get()
    }

    pub fn max_offset(&self) -> Pixels {
        clamp_min_zero(self.content_height.get() - self.viewport_height.get())
    }

    pub fn scroll_by(&self, delta: Pixels) -> bool {
        let next = clamp_offset(self.offset.get() + delta, self.max_offset());
        let changed = next != self.offset.get();
        self.offset.set(next);
        changed
    }

    pub(crate) fn clamp_to_layout(&self) {
        self.offset
            .set(clamp_offset(self.offset.get(), self.max_offset()));
    }

    pub(crate) fn record_layout(&self, content_height: Pixels, viewport_height: Pixels) {
        self.content_height.set(content_height);
        self.viewport_height.set(viewport_height);
    }
}

pub(crate) fn clamp_offset(offset: Pixels, max_offset: Pixels) -> Pixels {
    offset.max(Pixels::ZERO).min(clamp_min_zero(max_offset))
}

fn clamp_min_zero(value: Pixels) -> Pixels {
    value.max(Pixels::ZERO)
}

#[cfg(test)]
mod tests {
    use super::{EditorScrollState, clamp_offset};
    use gpui::px;

    #[test]
    fn max_offset_is_content_minus_viewport() {
        let state = EditorScrollState::default();
        state.record_layout(px(2000.), px(600.));
        assert_eq!(state.max_offset(), px(1400.));
    }

    #[test]
    fn max_offset_never_negative() {
        let state = EditorScrollState::default();
        state.record_layout(px(300.), px(600.));
        assert_eq!(state.max_offset(), px(0.));
    }

    #[test]
    fn scroll_by_clamps_and_reports_change() {
        let state = EditorScrollState::default();
        state.record_layout(px(1000.), px(400.));
        assert!(state.scroll_by(px(120.)));
        assert_eq!(state.offset(), px(120.));
        assert!(!state.scroll_by(px(0.)));
        state.scroll_by(px(-1000.));
        assert_eq!(state.offset(), px(0.));
        state.scroll_by(px(10000.));
        assert_eq!(state.offset(), px(600.));
    }

    #[test]
    fn clamp_offset_bounds() {
        assert_eq!(clamp_offset(px(-5.), px(100.)), px(0.));
        assert_eq!(clamp_offset(px(50.), px(100.)), px(50.));
        assert_eq!(clamp_offset(px(150.), px(100.)), px(100.));
        assert_eq!(clamp_offset(px(50.), px(-10.)), px(0.));
    }

    #[test]
    fn shrink_clamps_stored_offset() {
        let state = EditorScrollState::default();
        state.record_layout(px(1000.), px(400.));
        state.scroll_by(px(600.));
        state.record_layout(px(1000.), px(900.));
        state.clamp_to_layout();
        assert_eq!(state.offset(), px(100.));
    }
}
