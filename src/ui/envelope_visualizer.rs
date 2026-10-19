//! A visual envelope designer component that displays an ADSR envelope curve
//! and allows interactive editing by dragging control points
//! (port of `synth.ui.EnvelopeVisualizer`).
//!
//! The JavaFX original was a `Canvas` subclass holding bindable ADSR
//! properties; in immediate-mode egui the ADSR values live in the caller's
//! state ([`Adsr`]) and [`EnvelopeVisualizer::show`] draws the curve into an
//! allocated painter each frame, mutating the values while a control point is
//! dragged. The curve geometry (20 px padding, linear A/D/R segments) matches
//! the Java drawing code.
//!
//! The plot is four fixed-width segments (attack, decay, sustain hold,
//! release). Inside each time segment a handle's x is log time, so a handle
//! can never leave the plot and dragging one handle never moves another's
//! time. The same axis is shared with the A/D/R sliders
//! ([`time_to_position`] / [`position_to_time`]) and with the Java app and the
//! site.
//!
//! June's Logue - Visual Envelope Designer

use eframe::egui;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2};

use crate::ui::theme;

// Colors from the June's Logue theme
const BACKGROUND_COLOR: Color32 = theme::BLACK;
const GRID_COLOR: Color32 = theme::CHOCOLATE_COSMOS;
const CURVE_COLOR: Color32 = theme::ORANGE_PEEL;
const CONTROL_POINT_COLOR: Color32 = theme::BURNT_ORANGE;
const CONTROL_POINT_HIGHLIGHT: Color32 = theme::ORANGE_PEEL;
const TEXT_COLOR: Color32 = theme::WHITE_SMOKE;

const CONTROL_POINT_RADIUS: f32 = 6.0;

/// Padding for drawing area
const PADDING: f32 = 20.0;

/// Segment indices: attack, decay, sustain hold, release.
pub const ATTACK_SEGMENT: usize = 0;
pub const DECAY_SEGMENT: usize = 1;
pub const HOLD_SEGMENT: usize = 2;
pub const RELEASE_SEGMENT: usize = 3;

/// The A/D/R parameter range in seconds.
pub const T_MIN: f64 = 0.001;
pub const T_MAX: f64 = 5.0;

/// Share of the plot width taken by each segment (sums to 1).
const SEGMENT_FRACTION: [f64; 4] = [0.28, 0.28, 0.16, 0.28];

/// Time to 0..1 along a time segment (log, clamped to the parameter range).
pub fn time_to_fraction(t: f64) -> f64 {
    if t > T_MIN {
        ((t / T_MIN).ln() / (T_MAX / T_MIN).ln()).min(1.0)
    } else {
        0.0 // also catches NaN and non-positive times
    }
}

/// Exact inverse of [`time_to_fraction`].
pub fn fraction_to_time(f: f64) -> f64 {
    T_MIN * (T_MAX / T_MIN).powf(f.clamp(0.0, 1.0))
}

/// Slider position (0..1) for a time: the same taper as the graph.
pub fn time_to_position(t: f64) -> f64 {
    time_to_fraction(t)
}

/// Time in seconds for a slider position (0..1).
pub fn position_to_time(p: f64) -> f64 {
    fraction_to_time(p)
}

/// Start offset of a segment within a plot of width `plot_width`.
pub fn segment_start(segment: usize, plot_width: f64) -> f64 {
    SEGMENT_FRACTION[..segment].iter().sum::<f64>() * plot_width
}

/// End offset of a segment within a plot of width `plot_width`.
pub fn segment_end(segment: usize, plot_width: f64) -> f64 {
    segment_start(segment, plot_width) + SEGMENT_FRACTION[segment] * plot_width
}

/// Handle x offset (0..`plot_width`) for a time in a time segment.
pub fn time_to_x(segment: usize, t: f64, plot_width: f64) -> f64 {
    let x0 = segment_start(segment, plot_width);
    x0 + (segment_end(segment, plot_width) - x0) * time_to_fraction(t)
}

/// Time for an x offset in a time segment, clamped to the segment (the exact
/// inverse of [`time_to_x`]).
pub fn x_to_time(segment: usize, x: f64, plot_width: f64) -> f64 {
    let x0 = segment_start(segment, plot_width);
    fraction_to_time((x - x0) / (segment_end(segment, plot_width) - x0))
}

/// Short label for a time: "5ms", "250ms", "1.2s".
pub fn format_time(t: f64) -> String {
    if t < 0.9995 {
        format!("{}ms", (t * 1000.0).round())
    } else {
        format!("{t:.1}s")
    }
}

/// An ADSR parameter set edited by the visualizer (the equivalent of the four
/// bindable `DoubleProperty`s on the JavaFX component).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adsr {
    /// Attack time in seconds.
    pub attack: f64,
    /// Decay time in seconds.
    pub decay: f64,
    /// Sustain level in `[0, 1]`.
    pub sustain: f64,
    /// Release time in seconds.
    pub release: f64,
}

/// The draggable control points of the envelope curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPoint {
    None,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// The four control-point positions (attack, decay, sustain, release) in
/// screen coordinates for a widget occupying `rect`.
pub fn control_point_positions(rect: Rect, adsr: &Adsr) -> [Pos2; 4] {
    let draw_width = (rect.width() - 2.0 * PADDING) as f64;
    let draw_height = rect.height() - 2.0 * PADDING;
    let left = rect.left() + PADDING;
    let top = rect.top() + PADDING;
    let x = |segment, t| left + time_to_x(segment, t, draw_width) as f32;

    let decay_y = top + ((1.0 - adsr.sustain) as f32) * draw_height;
    [
        pos2(x(ATTACK_SEGMENT, adsr.attack), top), // Attack peak (top)
        pos2(x(DECAY_SEGMENT, adsr.decay), decay_y),
        // Sustain point: the end of the fixed-width hold segment
        pos2(left + segment_end(HOLD_SEGMENT, draw_width) as f32, decay_y),
        pos2(x(RELEASE_SEGMENT, adsr.release), top + draw_height), // Release to bottom
    ]
}

/// The control point nearest `pos` within the hit radius (ties go to the
/// later of A, D, S, R).
pub fn control_point_at(pos: Pos2, rect: Rect, adsr: &Adsr) -> ControlPoint {
    // Increased hit area for easier grabbing (radius * 3, as in Java)
    let mut best = (ControlPoint::None, CONTROL_POINT_RADIUS * 3.0);
    let points = [
        ControlPoint::Attack,
        ControlPoint::Decay,
        ControlPoint::Sustain,
        ControlPoint::Release,
    ];
    for (point, at) in points.into_iter().zip(control_point_positions(rect, adsr)) {
        let distance = pos.distance(at);
        if distance <= best.1 {
            best = (point, distance);
        }
    }
    best.0
}

/// The ADSR after dragging `point` so that its handle sits at `pointer + grab`
/// (`grab` is the handle minus the pointer at press, so a grab doesn't jump).
/// Times are clamped to the handle's own segment, sustain to `0..=1`.
pub fn apply_drag(point: ControlPoint, pointer: Pos2, grab: Vec2, rect: Rect, adsr: Adsr) -> Adsr {
    let draw_width = (rect.width() - 2.0 * PADDING) as f64;
    let draw_height = rect.height() - 2.0 * PADDING;
    let target = pointer + grab;
    let plot_x = (target.x - rect.left() - PADDING) as f64;
    let level = (1.0 - ((target.y - rect.top() - PADDING) / draw_height) as f64).clamp(0.0, 1.0);

    let mut out = adsr;
    match point {
        ControlPoint::Attack => out.attack = x_to_time(ATTACK_SEGMENT, plot_x, draw_width),
        ControlPoint::Decay => {
            out.decay = x_to_time(DECAY_SEGMENT, plot_x, draw_width);
            out.sustain = level;
        }
        // Only sustain level (Y position) - X position is fixed
        ControlPoint::Sustain => out.sustain = level,
        ControlPoint::Release => out.release = x_to_time(RELEASE_SEGMENT, plot_x, draw_width),
        ControlPoint::None => {}
    }
    out
}

/// Interactive ADSR envelope curve widget.
pub struct EnvelopeVisualizer {
    /// The control point currently being dragged, if any.
    dragged_point: ControlPoint,
    /// Handle position minus pointer position at press, so grabbing a handle
    /// off-centre doesn't move it.
    grab_offset: Vec2,
    /// Widget size (the Java component was a fixed 240x120 canvas).
    size: Vec2,
}

impl Default for EnvelopeVisualizer {
    fn default() -> Self {
        Self::new(240.0, 120.0)
    }
}

impl EnvelopeVisualizer {
    /// Creates a visualizer with the given canvas size in points.
    pub fn new(width: f32, height: f32) -> Self {
        EnvelopeVisualizer {
            dragged_point: ControlPoint::None,
            grab_offset: Vec2::ZERO,
            size: Vec2::new(width, height),
        }
    }

    /// Draws the envelope and handles control-point dragging.
    ///
    /// Returns `true` if the user changed any of the ADSR values this frame.
    pub fn show(&mut self, ui: &mut egui::Ui, adsr: &mut Adsr) -> bool {
        let (response, painter) = ui.allocate_painter(self.size, Sense::click_and_drag());
        let rect = response.rect;

        // Handle interaction first so the drawn curve reflects this frame's drag.
        let changed = self.handle_input(&response, rect, adsr);

        // Clear background
        painter.rect_filled(rect, 0.0, BACKGROUND_COLOR);

        self.draw_grid(&painter, rect);
        self.draw_envelope_curve(&painter, rect, adsr);
        self.draw_control_points(&painter, rect, adsr);
        self.draw_labels(&painter, rect, adsr);

        // Hand cursor when hovering a control point (Java set Cursor.HAND).
        if let Some(pos) = response.hover_pos()
            && control_point_at(pos, rect, adsr) != ControlPoint::None
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        changed
    }

    /// Port of the Java mouse-pressed/dragged/released handlers. Only the
    /// primary button drags.
    fn handle_input(&mut self, response: &egui::Response, rect: Rect, adsr: &mut Adsr) -> bool {
        if response.drag_started_by(egui::PointerButton::Primary) {
            // egui reports a drag only after the pointer has moved a little, so
            // hit-test where the button went down.
            let press = response
                .ctx
                .input(|i| i.pointer.press_origin())
                .or_else(|| response.interact_pointer_pos());
            if let Some(press) = press {
                self.dragged_point = control_point_at(press, rect, adsr);
                if self.dragged_point != ControlPoint::None {
                    let at = control_point_positions(rect, adsr)[self.dragged_point as usize - 1];
                    self.grab_offset = at - press;
                }
            }
        }
        if response.drag_stopped() {
            self.dragged_point = ControlPoint::None;
            return false;
        }
        if !response.dragged_by(egui::PointerButton::Primary)
            || self.dragged_point == ControlPoint::None
        {
            return false;
        }
        let Some(pos) = response.interact_pointer_pos() else {
            return false;
        };

        let before = *adsr;
        *adsr = apply_drag(self.dragged_point, pos, self.grab_offset, rect, before);
        *adsr != before
    }

    /// Draws the background grid (level tenths, and a line per decade inside
    /// each time segment), the segment dividers and the canvas border.
    fn draw_grid(&self, painter: &egui::Painter, rect: Rect) {
        let draw_width = rect.width() - 2.0 * PADDING;
        let draw_height = rect.height() - 2.0 * PADDING;
        let left = rect.left() + PADDING;
        let top = rect.top() + PADDING;
        let grid_stroke = Stroke::new(1.0, GRID_COLOR);

        // Vertical grid lines (time): 10 ms, 100 ms and 1 s in each time segment
        for segment in [ATTACK_SEGMENT, DECAY_SEGMENT, RELEASE_SEGMENT] {
            for t in [0.01, 0.1, 1.0] {
                let x = left + time_to_x(segment, t, draw_width as f64) as f32;
                painter.line_segment([pos2(x, top), pos2(x, top + draw_height)], grid_stroke);
            }
        }
        // Horizontal grid lines (level)
        for i in 0..=10 {
            let y = top + (i as f32 / 10.0) * draw_height;
            painter.line_segment([pos2(left, y), pos2(left + draw_width, y)], grid_stroke);
        }

        // Segment dividers
        let divider = Stroke::new(1.0, CONTROL_POINT_COLOR.gamma_multiply(0.55));
        for segment in 1..4 {
            let x = left + segment_start(segment, draw_width as f64) as f32;
            painter.extend(egui::Shape::dashed_line(
                &[pos2(x, top), pos2(x, top + draw_height)],
                divider,
                3.0,
                3.0,
            ));
        }

        // Draw border
        painter.rect_stroke(
            Rect::from_min_size(pos2(left, top), Vec2::new(draw_width, draw_height)),
            0.0,
            Stroke::new(2.0, CONTROL_POINT_COLOR),
            StrokeKind::Middle,
        );
    }

    /// Draws the four linear envelope segments (start->A->D->S->R).
    fn draw_envelope_curve(&self, painter: &egui::Painter, rect: Rect, adsr: &Adsr) {
        let [attack, decay, sustain, release] = control_point_positions(rect, adsr);
        let start = pos2(rect.left() + PADDING, rect.bottom() - PADDING);
        let stroke = Stroke::new(3.0, CURVE_COLOR);

        painter.line_segment([start, attack], stroke); // Start to Attack
        painter.line_segment([attack, decay], stroke); // Attack to Decay
        painter.line_segment([decay, sustain], stroke); // Sustain phase
        painter.line_segment([sustain, release], stroke); // Release phase
    }

    /// Draws the A/D/S/R control points, highlighting the dragged one.
    fn draw_control_points(&self, painter: &egui::Painter, rect: Rect, adsr: &Adsr) {
        let [attack, decay, sustain, release] = control_point_positions(rect, adsr);
        let points = [
            (attack, "A", ControlPoint::Attack),
            (decay, "D", ControlPoint::Decay),
            (sustain, "S", ControlPoint::Sustain),
            (release, "R", ControlPoint::Release),
        ];
        for (pos, label, point) in points {
            let highlighted = self.dragged_point == point;
            let fill_color = if highlighted {
                CONTROL_POINT_HIGHLIGHT
            } else {
                CONTROL_POINT_COLOR
            };
            painter.circle(
                pos,
                CONTROL_POINT_RADIUS,
                fill_color,
                Stroke::new(1.0, TEXT_COLOR),
            );
            painter.text(
                pos,
                Align2::CENTER_CENTER,
                label,
                FontId::monospace(10.0),
                TEXT_COLOR,
            );
        }
    }

    /// Draws each segment's name with its time, and the level axis labels.
    fn draw_labels(&self, painter: &egui::Painter, rect: Rect, adsr: &Adsr) {
        let font = FontId::monospace(9.0);
        let left = rect.left() + PADDING;
        let bottom = rect.bottom() - PADDING;
        let top = rect.top() + PADDING;
        let draw_width = (rect.width() - 2.0 * PADDING) as f64;

        // Segment labels
        let names = [
            format!("A {}", format_time(adsr.attack)),
            format!("D {}", format_time(adsr.decay)),
            "S".to_string(),
            format!("R {}", format_time(adsr.release)),
        ];
        for (segment, name) in names.into_iter().enumerate() {
            let centre =
                (segment_start(segment, draw_width) + segment_end(segment, draw_width)) / 2.0;
            painter.text(
                pos2(left + centre as f32, bottom + 13.0), // clear of the handles (radius 6) on the 0 line
                Align2::CENTER_CENTER,
                name,
                font.clone(),
                TEXT_COLOR,
            );
        }

        // Level axis labels
        painter.text(
            pos2(left - 12.0, bottom), // clear of the A handle at 1 ms
            Align2::CENTER_CENTER,
            "0",
            font.clone(),
            TEXT_COLOR,
        );
        painter.text(
            pos2(left - 12.0, top),
            Align2::CENTER_CENTER,
            "1",
            font,
            TEXT_COLOR,
        );
    }
}
