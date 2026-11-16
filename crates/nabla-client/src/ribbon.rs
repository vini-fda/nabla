//! Polylines as ribbons: the tessellation the gravity field's contour lines (`field.rs`) and
//! field lines (`streamlines.rs`) share, for strokes whose anti-aliasing is each fragment's
//! own distance from the line.
//!
//! A ribbon has two vertices at each point of the polyline, to either side of it along the
//! miter of the two segments that meet there. So its sides run parallel to every segment, a
//! fixed distance from it, and the signed distance across the ribbon, given to the two
//! vertices and interpolated between them, is exactly each fragment's distance to its
//! segment's line. The segments' quads share their vertices at the joins: nothing overlaps
//! there to blend twice, as separate quads with caps would, and nothing gaps.

use egui::{Rect, Vec2};

/// A miter no longer than this many times the ribbon's half width, where the polyline turns
/// sharply.
const MITER_LIMIT: f32 = 2.0;

/// The runs of a polyline inside `bounds`, cut at its edges. Values carried by the points are
/// interpolated at the cuts, as screen-space vertex attributes interpolate. In particular,
/// projective world coordinates and perspective depths remain correct there. A closed polyline
/// stays closed only when it is wholly inside; clipping never adds edges along `bounds`.
pub fn clip<T: Copy>(
    points: &[(Vec2, T)],
    bounds: Rect,
    closed: bool,
    lerp: impl Fn(T, T, f32) -> T,
) -> Vec<Vec<(Vec2, T)>> {
    let n = points.len();
    if n < 2 {
        return Vec::new();
    }
    let mut runs = Vec::new();
    let mut run: Vec<(Vec2, T)> = Vec::new();
    let mut whole = true;
    for (i, j) in segments(n, closed) {
        let ((a, carried_a), (b, carried_b)) = (points[i], points[j]);
        let Some((from, to)) = clip_segment(a, b, bounds) else {
            whole = false;
            if !run.is_empty() {
                runs.push(std::mem::take(&mut run));
            }
            continue;
        };
        whole &= from == 0.0 && to == 1.0;
        let at = |t: f32| {
            if t == 0.0 {
                (a, carried_a)
            } else if t == 1.0 {
                (b, carried_b)
            } else {
                (a + (b - a) * t, lerp(carried_a, carried_b, t))
            }
        };
        let (cut_a, cut_b) = (at(from), at(to));
        if run
            .last()
            .is_some_and(|(p, _)| (*p - cut_a.0).length() <= 1e-3)
        {
            run.push(cut_b);
        } else {
            if !run.is_empty() {
                runs.push(std::mem::take(&mut run));
            }
            run.extend([cut_a, cut_b]);
        }
    }
    if !run.is_empty() {
        runs.push(run);
    }
    if closed && whole {
        return vec![points.to_vec()];
    }
    // The closing segment may join the last run to the first. Both are open at the cut.
    if closed && runs.len() > 1 {
        let join =
            (runs[0][0].0 - runs[runs.len() - 1].last().expect("nonempty").0).length() <= 1e-3;
        if join {
            let first = runs.remove(0);
            runs.last_mut()
                .expect("another run")
                .extend(first.into_iter().skip(1));
        }
    }
    runs
}

/// The parameters at which segment `a..b` enters and leaves `bounds`, if it crosses it.
fn clip_segment(a: Vec2, b: Vec2, bounds: Rect) -> Option<(f32, f32)> {
    if ![a.x, a.y, b.x, b.y].into_iter().all(f32::is_finite) {
        return None;
    }
    let delta = b - a;
    let (mut enter, mut leave) = (0.0_f32, 1.0_f32);
    for axis in 0..2 {
        let (start, step) = (a[axis], delta[axis]);
        let (min, max) = (bounds.min[axis], bounds.max[axis]);
        if step.abs() < f32::EPSILON {
            if !(min..=max).contains(&start) {
                return None;
            }
            continue;
        }
        let first = (min - start) / step;
        let second = (max - start) / step;
        enter = enter.max(first.min(second));
        leave = leave.min(first.max(second));
        if enter > leave {
            return None;
        }
    }
    Some((enter, leave))
}

/// The points of a polyline on screen, each with whatever its vertices carry, less those so
/// near the one before that the pixels' rounding would turn the segment between them. A
/// `closed` polyline's last point is also kept clear of its first.
pub fn distinct<T: Copy>(
    points: impl IntoIterator<Item = (Vec2, T)>,
    closed: bool,
) -> Vec<(Vec2, T)> {
    const APART: f32 = 0.1;
    let mut out: Vec<(Vec2, T)> = Vec::new();
    for (p, carried) in points {
        if out.last().is_none_or(|(q, _)| (p - *q).length() > APART) {
            out.push((p, carried));
        }
    }
    while closed && out.len() > 1 && (out[0].0 - out[out.len() - 1].0).length() <= APART {
        out.pop();
    }
    out
}

/// For each point of the polyline through `points`, the vector from it to its vertex on the
/// ribbon's one side; the vertex on the other side is as far the other way. The ribbon
/// reaches `reach` to each side of every segment. A `closed` polyline joins its last point to
/// its first, which `points` holds the once, and so has a miter there as everywhere else.
/// Consecutive points must be apart: see [`distinct`].
pub fn offsets(points: &[Vec2], reach: f32, closed: bool) -> Vec<Vec2> {
    let n = points.len();
    let ring = closed && n > 2;
    let normal = |from: usize, to: usize| (points[to] - points[from]).normalized().rot90();
    (0..n)
        .map(|i| {
            let before = match i.checked_sub(1) {
                Some(j) => Some(normal(j, i)),
                None => ring.then(|| normal(n - 1, i)),
            };
            let after = if i + 1 < n {
                Some(normal(i, i + 1))
            } else {
                ring.then(|| normal(i, 0))
            };
            match (before, after) {
                (Some(a), Some(b)) if (a + b).length() > 1e-3 => {
                    let miter = (a + b).normalized();
                    miter * (reach / miter.dot(a).max(1.0 / MITER_LIMIT))
                }
                (Some(n), _) | (None, Some(n)) => n * reach,
                (None, None) => Vec2::ZERO,
            }
        })
        .collect()
}

/// The pairs of consecutive points' indices: the segments of a polyline of `n` points, with
/// the one from the last back to the first when it is `closed`.
pub fn segments(n: usize, closed: bool) -> impl Iterator<Item = (usize, usize)> {
    let ring = closed && n > 2;
    (1..n).map(|i| (i - 1, i)).chain(ring.then(|| (n - 1, 0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How far `p` is from the line through `a` and `b`, signed by its side.
    fn across(p: Vec2, a: Vec2, b: Vec2) -> f32 {
        let along = (b - a).normalized();
        let from = p - a;
        along.y * from.x - along.x * from.y
    }

    #[test]
    fn the_sides_run_the_reach_from_every_segment() {
        // A bending line, and a ring: no turn of either is sharp enough to cut its miter.
        let bend = [
            Vec2::new(10.0, 10.0),
            Vec2::new(60.0, 10.0),
            Vec2::new(100.0, 40.0),
            Vec2::new(100.0, 90.0),
        ];
        let square = [
            Vec2::new(0.0, 0.0),
            Vec2::new(40.0, 0.0),
            Vec2::new(40.0, 40.0),
            Vec2::new(0.0, 40.0),
        ];
        for (points, closed) in [(bend, false), (square, true)] {
            let offsets = offsets(&points, 3.0, closed);
            let mut count = 0;
            for (i, j) in segments(points.len(), closed) {
                for end in [i, j] {
                    // An open polyline's two ends are square to their one segment, and a
                    // miter at any other point is on both its segments' sides.
                    let plus = across(points[end] + offsets[end], points[i], points[j]);
                    let minus = across(points[end] - offsets[end], points[i], points[j]);
                    assert!((plus - 3.0).abs() < 1e-4, "{plus} at {end} of {i}-{j}");
                    assert!((minus + 3.0).abs() < 1e-4, "{minus} at {end} of {i}-{j}");
                }
                count += 1;
            }
            assert_eq!(count, if closed { 4 } else { 3 });
        }
    }

    #[test]
    fn a_sharp_turn_is_no_longer_than_the_limit() {
        // Doubling back at a hair's angle: the miter would reach far past the turn.
        let points = [
            Vec2::new(0.0, 0.0),
            Vec2::new(50.0, 0.0),
            Vec2::new(0.0, 1.0),
        ];
        let offsets = offsets(&points, 2.0, false);
        assert!(offsets[1].length() <= 2.0 * MITER_LIMIT + 1e-4);
        assert!(offsets.iter().all(|o| o.x.is_finite() && o.y.is_finite()));
    }

    #[test]
    fn points_too_near_the_last_are_left_out() {
        let at = |x: f32, y: f32| (Vec2::new(x, y), ());
        let open = distinct(
            [at(0.0, 0.0), at(0.01, 0.0), at(5.0, 0.0), at(5.0, 0.05)],
            false,
        );
        assert_eq!(open.len(), 2);
        // A ring given with its first point again at the end holds it the once.
        let ring = distinct(
            [at(0.0, 0.0), at(5.0, 0.0), at(5.0, 5.0), at(0.0, 0.0)],
            true,
        );
        assert_eq!(ring.len(), 3);
        assert_eq!(segments(ring.len(), true).last(), Some((2, 0)));
        // Two points make no ring.
        assert_eq!(segments(2, true).count(), 1);
    }

    #[test]
    fn clipping_keeps_crossings_and_interpolates_what_points_carry() {
        let bounds = Rect::from_min_size(Vec2::ZERO.to_pos2(), Vec2::splat(10.0));
        let points = [(Vec2::new(-1e6, 5.0), 0.0), (Vec2::new(1e6, 5.0), 2.0)];
        let runs = clip(&points, bounds, false, |a, b, t| a + (b - a) * t);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].len(), 2);
        assert_eq!(runs[0][0].0, Vec2::new(0.0, 5.0));
        assert_eq!(runs[0][1].0, Vec2::new(10.0, 5.0));
        assert!((runs[0][0].1 - 1.0).abs() < 1e-4);
        assert!((runs[0][1].1 - 1.0).abs() < 1e-4);
        // A ring around the viewport has no stroke inside it: do not outline the clip bounds.
        let outside = [
            (Vec2::new(-5.0, -5.0), ()),
            (Vec2::new(15.0, -5.0), ()),
            (Vec2::new(15.0, 15.0), ()),
            (Vec2::new(-5.0, 15.0), ()),
        ];
        assert!(clip(&outside, bounds, true, |a, _, _| a).is_empty());
    }

    #[test]
    fn clipping_a_ring_preserves_its_runs_without_closing_the_cuts() {
        let bounds = Rect::from_min_size(Vec2::ZERO.to_pos2(), Vec2::splat(10.0));
        let square = [
            (Vec2::new(2.0, -5.0), ()),
            (Vec2::new(8.0, -5.0), ()),
            (Vec2::new(8.0, 15.0), ()),
            (Vec2::new(2.0, 15.0), ()),
        ];
        let runs = clip(&square, bounds, true, |a, _, _| a);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].first().expect("cut").0, Vec2::new(8.0, 0.0));
        assert_eq!(runs[0].last().expect("cut").0, Vec2::new(8.0, 10.0));
        assert_eq!(runs[1].first().expect("cut").0, Vec2::new(2.0, 10.0));
        assert_eq!(runs[1].last().expect("cut").0, Vec2::new(2.0, 0.0));
        assert!(
            runs.iter()
                .flatten()
                .all(|(p, _)| bounds.contains(p.to_pos2()))
        );
        let inside = [
            (Vec2::new(1.0, 1.0), ()),
            (Vec2::new(9.0, 1.0), ()),
            (Vec2::new(9.0, 9.0), ()),
            (Vec2::new(1.0, 9.0), ()),
        ];
        assert_eq!(
            clip(&inside, bounds, true, |a, _, _| a),
            vec![inside.to_vec()]
        );
    }
}
