use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cursive::backend::Backend;
use cursive::event::Event;
use cursive::style::{Color, ColorPair, Effect};
use cursive::Vec2;

/// Wraps a cursive backend so that a full repaint can be forced.
///
/// Persistent backends (like crossterm) only redraw cells that cursive believes
/// have changed since the last frame, so anything that scribbles on the terminal
/// behind cursive's back (e.g. a dependency writing to stderr) is never repaired.
/// While `force_repaint` is set, the backend reports itself as non-persistent,
/// which makes cursive redraw every cell on the next refresh.
pub(crate) struct RepaintableBackend {
    inner: Box<dyn Backend>,
    force_repaint: Arc<AtomicBool>,
}

impl RepaintableBackend {
    pub(crate) fn new(inner: Box<dyn Backend>, force_repaint: Arc<AtomicBool>) -> Self {
        Self {
            inner,
            force_repaint,
        }
    }
}

impl Backend for RepaintableBackend {
    fn poll_event(&mut self) -> Option<Event> {
        self.inner.poll_event()
    }

    fn set_title(&mut self, title: String) {
        self.inner.set_title(title)
    }

    fn refresh(&mut self) {
        self.inner.refresh()
    }

    fn has_colors(&self) -> bool {
        self.inner.has_colors()
    }

    fn screen_size(&self) -> Vec2 {
        self.inner.screen_size()
    }

    fn move_to(&self, pos: Vec2) {
        self.inner.move_to(pos)
    }

    fn print(&self, text: &str) {
        self.inner.print(text)
    }

    fn clear(&self, color: Color) {
        self.inner.clear(color)
    }

    fn set_color(&self, colors: ColorPair) -> ColorPair {
        self.inner.set_color(colors)
    }

    fn set_effect(&self, effect: Effect) {
        self.inner.set_effect(effect)
    }

    fn is_persistent(&self) -> bool {
        !self.force_repaint.load(Ordering::Relaxed) && self.inner.is_persistent()
    }

    fn unset_effect(&self, effect: Effect) {
        self.inner.unset_effect(effect)
    }

    fn name(&self) -> &str {
        self.inner.name()
    }
}
