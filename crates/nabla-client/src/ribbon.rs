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

use egui::Vec2;

/// A miter no longer than this many times the ribbon's half width, where the polyline turns
/// sharply.
const MITER_LIMIT: f32 = 2.0;

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
}
