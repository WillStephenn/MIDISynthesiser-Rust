//! A visual envelope designer component that displays an ADSR envelope curve
//! and allows interactive editing by dragging control points
//! (port of `synth.ui.EnvelopeVisualizer`).
//!
//! The JavaFX original was a `Canvas` subclass holding bindable ADSR
//! properties; in immediate-mode egui the ADSR values live in the caller's
//! state ([`Adsr`]) and [`EnvelopeVisualizer::show`] draws the curve into an
//! allocated painter each frame, mutating the values while a control point is
//! dragged. The curve geometry (20 px padding, 3 s time scale, fixed 80 px
//! sustain segment, linear A/D/R segments) matches the Java drawing code.
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

/// Length of the fixed sustain-phase segment, in pixels (matches the Java
/// `sustainX = decayX + 80` visualisation).
const SUSTAIN_SEGMENT_PX: f32 = 80.0;

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
enum ControlPoint {
    None,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Interactive ADSR envelope curve widget.
pub struct EnvelopeVisualizer {
    /// Time scaling for visualization (max time to display).
    max_time_seconds: f64,
    /// The control point currently being dragged, if any.
    dragged_point: ControlPoint,
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
            max_time_seconds: 3.0,
            dragged_point: ControlPoint::None,
            size: Vec2::new(width, height),
        }
    }

    /// Utility method to set the max time scale.
    #[allow(dead_code)]
    pub fn set_max_time_seconds(&mut self, max_time: f64) {
        self.max_time_seconds = max_time;
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
        self.draw_labels(&painter, rect);

        // Hand cursor when hovering a control point (Java set Cursor.HAND).
        if let Some(pos) = response.hover_pos()
            && self.control_point_at(pos, rect, adsr) != ControlPoint::None
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        changed
    }

    /// Computes the four control-point positions in screen coordinates.
    /// Mirrors the geometry in the Java `drawEnvelopeCurve`/`getControlPointAt`.
    fn control_point_positions(&self, rect: Rect, adsr: &Adsr) -> [Pos2; 4] {
        let draw_width = rect.width() - 2.0 * PADDING;
        let draw_height = rect.height() - 2.0 * PADDING;
        let left = rect.left() + PADDING;
        let top = rect.top() + PADDING;
        let max = self.max_time_seconds;

        let attack_x = left + ((adsr.attack / max) as f32) * draw_width;
        let attack_y = top; // Attack peak (top)

        let decay_x = left + (((adsr.attack + adsr.decay) / max) as f32) * draw_width;
        let decay_y = top + ((1.0 - adsr.sustain) as f32) * draw_height;

        // Sustain point - positioned at the end of the fixed sustain segment
        let sustain_x = decay_x + SUSTAIN_SEGMENT_PX;
        let sustain_y = decay_y;

        // Release point - clamped to the visible area
        let release_x =
            (sustain_x + ((adsr.release / max) as f32) * draw_width).min(left + draw_width);
        let release_y = top + draw_height; // Release to bottom

        [
            pos2(attack_x, attack_y),
            pos2(decay_x, decay_y),
            pos2(sustain_x, sustain_y),
            pos2(release_x, release_y),
        ]
    }

    /// Returns the control point under `pos`, if any. Overlapping points are
    /// checked in reverse order, as in the Java original.
    fn control_point_at(&self, pos: Pos2, rect: Rect, adsr: &Adsr) -> ControlPoint {
        let [attack, decay, sustain, release] = self.control_point_positions(rect, adsr);
        // Increased hit area for easier grabbing (radius * 3, as in Java)
        let hit = CONTROL_POINT_RADIUS * 3.0;
        if pos.distance(release) <= hit {
            ControlPoint::Release
        } else if pos.distance(sustain) <= hit {
            ControlPoint::Sustain
        } else if pos.distance(decay) <= hit {
            ControlPoint::Decay
        } else if pos.distance(attack) <= hit {
            ControlPoint::Attack
        } else {
            ControlPoint::None
        }
    }

    /// Port of the Java mouse-pressed/dragged/released handlers.
    fn handle_input(&mut self, response: &egui::Response, rect: Rect, adsr: &mut Adsr) -> bool {
        if response.drag_started()
            && let Some(pos) = response.interact_pointer_pos()
        {
            self.dragged_point = self.control_point_at(pos, rect, adsr);
        }
        if response.drag_stopped() {
            self.dragged_point = ControlPoint::None;
            return false;
        }
        if !response.dragged() || self.dragged_point == ControlPoint::None {
            return false;
        }
        let Some(pos) = response.interact_pointer_pos() else {
            return false;
        };

        let draw_width = rect.width() - 2.0 * PADDING;
        let draw_height = rect.height() - 2.0 * PADDING;

        // Convert screen coordinates to normalised envelope coordinates
        // (Y axis flipped), clamped to [0, 1].
        let normalized_x =
            (((pos.x - rect.left() - PADDING) / draw_width) as f64).clamp(0.0, 1.0);
        let normalized_y =
            (1.0 - ((pos.y - rect.top() - PADDING) / draw_height) as f64).clamp(0.0, 1.0);

        let max = self.max_time_seconds;
        let before = *adsr;
        match self.dragged_point {
            ControlPoint::Attack => {
                // Attack time is based on X position, limited to 30% of total time
                let new_attack_time = normalized_x * max * 0.3;
                adsr.attack = new_attack_time.max(0.001);
            }
            ControlPoint::Decay => {
                // Decay time and sustain level
                adsr.decay = ((normalized_x * max) - adsr.attack).max(0.001);
                adsr.sustain = normalized_y;
            }
            ControlPoint::Sustain => {
                // Only sustain level (Y position) - X position is fixed
                adsr.sustain = normalized_y;
            }
            ControlPoint::Release => {
                // Release time is measured from the end of the fixed-width
                // sustain segment; only update when dragging right of it.
                let sustain_end_x = (adsr.attack + adsr.decay) / max;
                let sustain_visualization_width = (SUSTAIN_SEGMENT_PX / draw_width) as f64;
                let release_start_x = sustain_end_x + sustain_visualization_width;
                if normalized_x > release_start_x {
                    adsr.release = ((normalized_x - release_start_x) * max).max(0.001);
                }
            }
            ControlPoint::None => {}
        }
        *adsr != before
    }

    /// Draws the 10x10 background grid and the canvas border.
    fn draw_grid(&self, painter: &egui::Painter, rect: Rect) {
        let draw_width = rect.width() - 2.0 * PADDING;
        let draw_height = rect.height() - 2.0 * PADDING;
        let left = rect.left() + PADDING;
        let top = rect.top() + PADDING;
        let grid_stroke = Stroke::new(1.0, GRID_COLOR);

        // Vertical grid lines (time)
        for i in 0..=10 {
            let x = left + (i as f32 / 10.0) * draw_width;
            painter.line_segment([pos2(x, top), pos2(x, top + draw_height)], grid_stroke);
        }
        // Horizontal grid lines (level)
        for i in 0..=10 {
            let y = top + (i as f32 / 10.0) * draw_height;
            painter.line_segment([pos2(left, y), pos2(left + draw_width, y)], grid_stroke);
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
        let [attack, decay, sustain, release] = self.control_point_positions(rect, adsr);
        let start = pos2(rect.left() + PADDING, rect.bottom() - PADDING);
        let stroke = Stroke::new(3.0, CURVE_COLOR);

        painter.line_segment([start, attack], stroke); // Start to Attack
        painter.line_segment([attack, decay], stroke); // Attack to Decay
        painter.line_segment([decay, sustain], stroke); // Sustain phase
        painter.line_segment([sustain, release], stroke); // Release phase
    }

    /// Draws the A/D/S/R control points, highlighting the dragged one.
    fn draw_control_points(&self, painter: &egui::Painter, rect: Rect, adsr: &Adsr) {
        let [attack, decay, sustain, release] = self.control_point_positions(rect, adsr);
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

    /// Draws the time/level axis labels.
    fn draw_labels(&self, painter: &egui::Painter, rect: Rect) {
        let font = FontId::monospace(9.0);
        let left = rect.left() + PADDING;
        let right = rect.right() - PADDING;
        let bottom = rect.bottom() - PADDING;
        let top = rect.top() + PADDING;

        // Time axis labels
        painter.text(
            pos2(left - 5.0, bottom + 8.0),
            Align2::LEFT_CENTER,
            "0s",
            font.clone(),
            TEXT_COLOR,
        );
        painter.text(
            pos2(right, bottom + 8.0),
            Align2::RIGHT_CENTER,
            format!("{:.1}s", self.max_time_seconds),
            font.clone(),
            TEXT_COLOR,
        );

        // Level axis labels
        painter.text(
            pos2(left - 8.0, bottom),
            Align2::CENTER_CENTER,
            "0",
            font.clone(),
            TEXT_COLOR,
        );
        painter.text(
            pos2(left - 8.0, top),
            Align2::CENTER_CENTER,
            "1",
            font,
            TEXT_COLOR,
        );
    }
}
