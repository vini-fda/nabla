//! The ship's collision hull: the shape of a ship's collider, in place of a ball of the hull
//! radius.
//!
//! It is the ship's own outline seen from the plane it flies in: slice the 3D model the
//! client draws (`nabla-client`'s `ship3d`, the glTF `nabla_starship.glb`, whose nose points
//! along +y) with the plane of the trajectory, which the hull's axis lies in, and [`HULL`]
//! is the convex hull of the points of that slice, so it contains every point of the model
//! in the plane. The client slices the model again and checks the two agree (`ship3d`'s
//! tests).
//!
//! The points are counterclockwise, in units of the hull radius `rho` (a world's
//! [`hull_radius`](crate::world::World::hull_radius): the distance from the ship's center to
//! its nose, which the drawn model is scaled to), in the ship's own frame, with the nose
//! along +x, so a ship of hull radius `rho` collides through the points that [`points`]
//! scales to it, turned by its heading.

/// The hull of the ship's slice, counterclockwise, nose along +x, `rho`-scaled.
pub const HULL: [[f32; 2]; 35] = [
    [-0.844_259_3, -0.137_839_8],
    [-0.591_242_3, -0.213_484_1],
    [-0.357_354_3, -0.273_449],
    [-0.262_727_2, -0.278_938_2],
    [-0.149_369_7, -0.269_954_1],
    [-0.054_251_61, -0.258_456_2],
    [0.182_028_6, -0.225_837_3],
    [0.477_221_8, -0.182_179_6],
    [0.507_612_1, -0.177_593_3],
    [0.828_770_6, -0.122_218_7],
    [0.869_143_5, -0.114_866_3],
    [0.908_844_5, -0.105_215_2],
    [0.930_318_7, -0.098_449_01],
    [0.942_521_9, -0.093_627_14],
    [0.984_179_6, -0.072_269_69],
    [0.996_665_3, -0.061_190_76],
    [0.999_093_3, -0.049_907_94],
    [0.999_417_8, -0.032_424_49],
    [0.998_821, 0.055_698_08],
    [0.995_392_1, 0.063_436_5],
    [0.986_388_2, 0.069_479_84],
    [0.940_714_7, 0.095_658_9],
    [0.916_598_8, 0.103_513_9],
    [0.907_474_3, 0.106_406],
    [0.897_931_1, 0.108_834],
    [0.506_834_3, 0.178_152_9],
    [0.467_993_3, 0.184_293_8],
    [0.276_710_9, 0.212_836_8],
    [-0.138_432_4, 0.268_763_3],
    [-0.199_650_7, 0.273_998_8],
    [-0.249_086_2, 0.276_446_6],
    [-0.360_380_7, 0.272_900_3],
    [-0.590_157_6, 0.213_934_6],
    [-0.843_881_4, 0.141_336_8],
    [-0.843_932_2, 0.127_100_4],
];

/// The hull of a ship of hull radius `rho`: what its collider is built from.
pub fn points(rho: f32) -> Vec<[f32; 2]> {
    HULL.iter().map(|&[x, y]| [rho * x, rho * y]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Twice the signed area of the triangle `o, a, b`.
    fn cross(o: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    }

    #[test]
    fn the_hull_is_a_counterclockwise_convex_polygon() {
        let n = HULL.len();
        for i in 0..n {
            let turn = cross(HULL[i], HULL[(i + 1) % n], HULL[(i + 2) % n]);
            assert!(turn > 0.0, "vertex {i} turns {turn}");
        }
    }

    #[test]
    fn the_hull_spans_the_ship_from_nose_to_tail() {
        let span = |axis: usize| {
            (
                HULL.iter().map(|p| p[axis]).fold(f32::MIN, f32::max),
                HULL.iter().map(|p| p[axis]).fold(f32::MAX, f32::min),
            )
        };
        let (nose, tail) = span(0);
        assert!(nose > 0.99 && tail > -1.0, "nose {nose} tail {tail}");
        let (upper, lower) = span(1);
        assert!(upper < 1.0 && lower > -1.0, "sides {lower} {upper}");
    }

    #[test]
    fn points_scale_with_the_hull_radius() {
        let points = points(2.0);
        assert_eq!(points.len(), HULL.len());
        for (scaled, &[x, y]) in points.iter().zip(&HULL) {
            assert!((scaled[0] - 2.0 * x).abs() < 1e-6);
            assert!((scaled[1] - 2.0 * y).abs() < 1e-6);
        }
    }
}
