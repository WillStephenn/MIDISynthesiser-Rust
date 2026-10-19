//! Tests for the envelope graph's time axis and drag mapping: segments tile
//! the plot, forward and inverse mappings agree, and a handle can never leave
//! its own segment. Pure maths (no egui context needed).

use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};
use midi_synthesiser::ui::envelope_visualizer::{
    ATTACK_SEGMENT, Adsr, ControlPoint, DECAY_SEGMENT, HOLD_SEGMENT, RELEASE_SEGMENT, T_MAX, T_MIN,
    apply_drag, control_point_at, control_point_positions, format_time, fraction_to_time,
    position_to_time, segment_end, segment_start, time_to_fraction, time_to_position, time_to_x,
    x_to_time,
};

const PLOT_WIDTH: f64 = 200.0;
const TIME_SEGMENTS: [usize; 3] = [ATTACK_SEGMENT, DECAY_SEGMENT, RELEASE_SEGMENT];

fn near(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

/// The widget rectangle of the 240x120 canvas, placed away from the origin so
/// the tests also cover the screen-offset arithmetic.
fn canvas() -> Rect {
    Rect::from_min_size(pos2(100.0, 50.0), vec2(240.0, 120.0))
}

const DEFAULT: Adsr = Adsr {
    attack: 0.005,
    decay: 0.1,
    sustain: 0.4,
    release: 0.4,
};

#[test]
fn segments_tile_the_plot() {
    assert!(near(segment_start(ATTACK_SEGMENT, PLOT_WIDTH), 0.0, 1e-9));
    assert!(near(
        segment_end(RELEASE_SEGMENT, PLOT_WIDTH),
        PLOT_WIDTH,
        1e-9
    ));
    for i in 0..3 {
        assert!(near(
            segment_end(i, PLOT_WIDTH),
            segment_start(i + 1, PLOT_WIDTH),
            1e-9
        ));
    }
    let hold = segment_end(HOLD_SEGMENT, PLOT_WIDTH) - segment_start(HOLD_SEGMENT, PLOT_WIDTH);
    let attack =
        segment_end(ATTACK_SEGMENT, PLOT_WIDTH) - segment_start(ATTACK_SEGMENT, PLOT_WIDTH);
    assert!(hold < attack, "the sustain hold is the narrow segment");
}

#[test]
fn range_ends_land_on_segment_ends() {
    for s in TIME_SEGMENTS {
        assert!(near(
            time_to_x(s, T_MIN, PLOT_WIDTH),
            segment_start(s, PLOT_WIDTH),
            1e-9
        ));
        assert!(near(
            time_to_x(s, T_MAX, PLOT_WIDTH),
            segment_end(s, PLOT_WIDTH),
            1e-9
        ));
    }
}

#[test]
fn forward_then_inverse_round_trips() {
    for s in TIME_SEGMENTS {
        for k in 0..=200 {
            let t = T_MIN * (T_MAX / T_MIN).powf(k as f64 / 200.0);
            let back = x_to_time(s, time_to_x(s, t, PLOT_WIDTH), PLOT_WIDTH);
            assert!(near(back / t, 1.0, 1e-9), "{t} -> {back}");
        }
    }
}

#[test]
fn inverse_then_forward_round_trips() {
    for s in TIME_SEGMENTS {
        let (x0, x1) = (segment_start(s, PLOT_WIDTH), segment_end(s, PLOT_WIDTH));
        for k in 0..=50 {
            let x = x0 + (x1 - x0) * k as f64 / 50.0;
            let back = time_to_x(s, x_to_time(s, x, PLOT_WIDTH), PLOT_WIDTH);
            assert!(near(back, x, 1e-7), "{x} -> {back}");
        }
    }
}

#[test]
fn pointer_outside_a_segment_clamps_to_the_parameter_range() {
    for s in TIME_SEGMENTS {
        let (x0, x1) = (segment_start(s, PLOT_WIDTH), segment_end(s, PLOT_WIDTH));
        assert!(near(x_to_time(s, x0 - 500.0, PLOT_WIDTH), T_MIN, 1e-12));
        assert!(near(x_to_time(s, x1 + 500.0, PLOT_WIDTH), T_MAX, 1e-12));
        assert!(time_to_x(s, 1e6, PLOT_WIDTH) <= x1 + 1e-9);
        assert!(time_to_x(s, -3.0, PLOT_WIDTH) >= x0);
        assert!(time_to_x(s, f64::NAN, PLOT_WIDTH) >= x0);
    }
}

#[test]
fn short_times_get_real_width() {
    // The 1 to 50 ms attacks that matter musically take at least a third of the segment
    assert!(time_to_fraction(0.05) > 0.3);
    // Every decade is the same width
    assert!(near(
        time_to_fraction(0.01) - time_to_fraction(0.001),
        time_to_fraction(1.0) - time_to_fraction(0.1),
        1e-9
    ));
}

#[test]
fn slider_taper_matches_the_graph() {
    assert!(near(position_to_time(0.0), T_MIN, 1e-12));
    assert!(near(position_to_time(1.0), T_MAX, 1e-12));
    assert!(near(position_to_time(0.5), (T_MIN * T_MAX).sqrt(), 1e-12));
    for t in [0.001, 0.005, 0.1, 0.4, 3.0, 5.0] {
        assert!(near(position_to_time(time_to_position(t)) / t, 1.0, 1e-9));
        assert_eq!(time_to_position(t), time_to_fraction(t));
        assert_eq!(position_to_time(0.37), fraction_to_time(0.37));
    }
}

#[test]
fn time_labels() {
    assert_eq!(format_time(0.005), "5ms");
    assert_eq!(format_time(0.4), "400ms");
    assert_eq!(format_time(1.0), "1.0s");
    assert_eq!(format_time(5.0), "5.0s");
}

#[test]
fn handles_stay_inside_the_plot_for_any_parameters() {
    let rect = canvas();
    let (left, right) = (rect.left() + 20.0, rect.right() - 20.0);
    for &t in &[T_MIN, 0.05, 1.0, T_MAX] {
        for &s in &[0.0, 0.5, 1.0] {
            let adsr = Adsr {
                attack: t,
                decay: t,
                sustain: s,
                release: t,
            };
            for p in control_point_positions(rect, &adsr) {
                assert!(
                    p.x >= left - 1e-3 && p.x <= right + 1e-3,
                    "{p:?} outside the plot"
                );
                assert!(p.y >= rect.top() + 20.0 - 1e-3 && p.y <= rect.bottom() - 20.0 + 1e-3);
            }
        }
    }
}

#[test]
fn dragging_far_outside_never_leaves_the_segment() {
    let rect = canvas();
    let far = [
        pos2(-5000.0, -5000.0),
        pos2(5000.0, 5000.0),
        pos2(5000.0, -5000.0),
        pos2(-5000.0, 5000.0),
    ];
    for point in [
        ControlPoint::Attack,
        ControlPoint::Decay,
        ControlPoint::Sustain,
        ControlPoint::Release,
    ] {
        for pointer in far {
            let out = apply_drag(point, pointer, Vec2::ZERO, rect, DEFAULT);
            assert!(
                (T_MIN..=T_MAX).contains(&out.attack),
                "{point:?} {pointer:?} {out:?}"
            );
            assert!((T_MIN..=T_MAX).contains(&out.decay));
            assert!((T_MIN..=T_MAX).contains(&out.release));
            assert!((0.0..=1.0).contains(&out.sustain));
        }
    }
    // Far right is exactly the maximum, far left the minimum
    assert_eq!(
        apply_drag(
            ControlPoint::Attack,
            pos2(5000.0, 0.0),
            Vec2::ZERO,
            rect,
            DEFAULT
        )
        .attack,
        T_MAX
    );
    assert_eq!(
        apply_drag(
            ControlPoint::Release,
            pos2(-5000.0, 0.0),
            Vec2::ZERO,
            rect,
            DEFAULT
        )
        .release,
        T_MIN
    );
}

#[test]
fn dragging_one_handle_changes_only_its_own_parameters() {
    let rect = canvas();
    let target = pos2(rect.left() + 70.0, rect.top() + 60.0);
    let a = apply_drag(ControlPoint::Attack, target, Vec2::ZERO, rect, DEFAULT);
    assert_eq!(
        (a.decay, a.sustain, a.release),
        (DEFAULT.decay, DEFAULT.sustain, DEFAULT.release)
    );
    let d = apply_drag(ControlPoint::Decay, target, Vec2::ZERO, rect, DEFAULT);
    assert_eq!((d.attack, d.release), (DEFAULT.attack, DEFAULT.release));
    let s = apply_drag(ControlPoint::Sustain, target, Vec2::ZERO, rect, DEFAULT);
    assert_eq!(
        (s.attack, s.decay, s.release),
        (DEFAULT.attack, DEFAULT.decay, DEFAULT.release)
    );
    let r = apply_drag(ControlPoint::Release, target, Vec2::ZERO, rect, DEFAULT);
    assert_eq!(
        (r.attack, r.decay, r.sustain),
        (DEFAULT.attack, DEFAULT.decay, DEFAULT.sustain)
    );
}

#[test]
fn a_dragged_handle_follows_the_pointer_without_jumping_on_grab() {
    let rect = canvas();
    for point in [
        ControlPoint::Attack,
        ControlPoint::Decay,
        ControlPoint::Release,
    ] {
        let at = control_point_positions(rect, &DEFAULT)[point as usize - 1];
        // Press 9 px off the handle centre; the grab offset is handle minus pointer
        let press = at + vec2(7.0, 6.0);
        let grab = at - press;
        let unchanged = apply_drag(point, press, grab, rect, DEFAULT);
        assert!(
            near(unchanged.attack / DEFAULT.attack, 1.0, 1e-4),
            "{point:?} jumped on grab: {unchanged:?}"
        );
        assert!(
            near(unchanged.decay / DEFAULT.decay, 1.0, 1e-4),
            "{point:?} jumped on grab: {unchanged:?}"
        );
        assert!(
            near(unchanged.release / DEFAULT.release, 1.0, 1e-4),
            "{point:?} jumped on grab: {unchanged:?}"
        );
        // Move the pointer 20 px (left for release, which is near the right edge): the handle ends 20 px away
        let dx = if point == ControlPoint::Release {
            -20.0
        } else {
            20.0
        };
        let moved = apply_drag(point, press + vec2(dx, 0.0), grab, rect, DEFAULT);
        let new_at = control_point_positions(rect, &moved)[point as usize - 1];
        assert!(
            near(new_at.x as f64, (at.x + dx) as f64, 1e-2),
            "{point:?}: {} vs {}",
            new_at.x,
            at.x + dx
        );
    }
}

#[test]
fn nearest_handle_is_grabbed_not_the_first_in_a_fixed_order() {
    let rect = canvas();
    // Sustain 1 puts D level with A, and a long attack with a short decay brings them a few pixels apart;
    // pressing right on A must grab A, not D
    let adsr = Adsr {
        attack: 4.0,
        decay: 0.002,
        sustain: 1.0,
        ..DEFAULT
    };
    let [a, d, ..] = control_point_positions(rect, &adsr);
    assert!(a.distance(d) < 18.0, "the test needs overlapping hit areas");
    assert_eq!(control_point_at(a, rect, &adsr), ControlPoint::Attack);
    assert_eq!(control_point_at(d, rect, &adsr), ControlPoint::Decay);
    // Release overlapping the sustain handle at sustain 0: each is grabbed from its own centre
    let adsr = Adsr {
        sustain: 0.0,
        release: 0.002,
        ..DEFAULT
    };
    let [_, _, s, r] = control_point_positions(rect, &adsr);
    assert_eq!(
        control_point_at(s, rect, &adsr),
        if s.distance(r) == 0.0 {
            ControlPoint::Release
        } else {
            ControlPoint::Sustain
        }
    );
    assert_eq!(control_point_at(r, rect, &adsr), ControlPoint::Release);
    // Nowhere near a handle
    assert_eq!(
        control_point_at(Pos2::new(0.0, 0.0), rect, &DEFAULT),
        ControlPoint::None
    );
}
