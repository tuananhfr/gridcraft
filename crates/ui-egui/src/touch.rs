//! Touch on the sheet: whether the user is on a touch screen right now, and the fling that keeps a
//! one-finger pan going after the finger lifts.

use egui::{Event, Vec2};

/// Radius, in points, around the selection's bottom-right corner that grabs the range handle.
pub const HANDLE_HIT_RADIUS: f32 = 22.0;
/// Radius of the painted range handle.
pub const HANDLE_RADIUS: f32 = 6.0;

/// Fling speeds, in points per second.
const FLING_MIN_SPEED: f32 = 250.0;
const FLING_STOP_SPEED: f32 = 20.0;
/// Fraction of the fling speed lost per second.
const FLING_DECAY: f32 = 4.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TouchInput {
    /// The last pointer input came from a finger.
    pub active: bool,
    /// Pan velocity carried on after release (screen points per second).
    fling: Option<Vec2>,
}

impl TouchInput {
    /// Feeds one frame's events. eframe sends `Event::Touch` next to the pointer events a finger
    /// makes, and a press can arrive a frame before its touch, so only a mouse move or wheel
    /// without touches switches back to mouse.
    pub fn observe(&mut self, events: &[Event]) {
        if events.iter().any(|e| matches!(e, Event::Touch { .. })) {
            self.active = true;
        } else if events.iter().any(|e| matches!(e, Event::PointerMoved(_) | Event::MouseWheel { .. })) {
            self.active = false;
            self.fling = None;
        }
    }

    /// Starts a fling from the release velocity of a pan, if it was quick enough.
    pub fn release(&mut self, velocity: Vec2) {
        self.fling = (velocity.is_finite() && velocity.length() >= FLING_MIN_SPEED).then_some(velocity);
    }

    pub fn stop(&mut self) {
        self.fling = None;
    }

    pub fn flinging(&self) -> bool {
        self.fling.is_some()
    }

    /// The screen distance the fling covers in `dt` seconds, slowing it down; zero once stopped.
    pub fn fling_step(&mut self, dt: f32) -> Vec2 {
        let Some(v) = self.fling else { return Vec2::ZERO };
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.1) } else { 0.0 };
        let next = v * (1.0 - FLING_DECAY * dt).max(0.0);
        self.fling = (next.length() >= FLING_STOP_SPEED).then_some(next);
        v * dt
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Modifiers, PointerButton, TouchDeviceId, TouchId, TouchPhase, pos2, vec2};

    fn touch(phase: TouchPhase) -> Event {
        Event::Touch { device_id: TouchDeviceId(1), id: TouchId(0), phase, pos: pos2(10.0, 10.0), force: None }
    }

    #[test]
    fn a_finger_is_touch_until_the_mouse_moves() {
        let mut t = TouchInput::default();
        t.observe(&[Event::PointerMoved(pos2(1.0, 1.0)), touch(TouchPhase::Start)]);
        assert!(t.active);
        // The press eframe sends from `pointerdown` comes alone, and the release is followed by
        // `PointerGone`: neither means a mouse.
        let press = Event::PointerButton { pos: pos2(1.0, 1.0), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE };
        t.observe(&[press]);
        t.observe(&[Event::PointerGone]);
        assert!(t.active);
        t.observe(&[Event::PointerMoved(pos2(5.0, 5.0))]);
        assert!(!t.active);
    }

    #[test]
    fn a_fling_slows_down_and_stops() {
        let mut t = TouchInput::default();
        t.release(vec2(100.0, 0.0));
        assert!(!t.flinging(), "a slow release doesn't fling");
        t.release(vec2(0.0, -1200.0));
        let first = t.fling_step(1.0 / 60.0);
        assert!(first.y < 0.0);
        let mut steps = 0;
        while t.flinging() && steps < 1000 {
            let d = t.fling_step(1.0 / 60.0);
            assert!(d.y.abs() <= first.y.abs() + 1e-3);
            steps += 1;
        }
        assert!(!t.flinging() && steps < 1000);
        t.release(vec2(f32::NAN, 0.0));
        assert!(!t.flinging());
    }
}
