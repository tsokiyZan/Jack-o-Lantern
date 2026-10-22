//! Vanilla `Leashable` spring math: the constants and `Wrench` record, plus
//! `computeDampenedSpringInteraction`.

use pumpkin_util::math::vector3::Vector3;

/// Vanilla `Leashable.LEASH_TOO_FAR_DIST`.
pub const LEASH_TOO_FAR_DIST: f64 = 12.0;
/// Vanilla `Leashable.LEASH_ELASTIC_DIST`.
pub const LEASH_ELASTIC_DIST: f64 = 6.0;
/// Vanilla `Leashable.MAXIMUM_ALLOWED_LEASHED_DIST`.
pub const MAXIMUM_ALLOWED_LEASHED_DIST: f64 = 16.0;
/// Vanilla `Leashable.AXIS_SPECIFIC_ELASTICITY`.
pub const AXIS_SPECIFIC_ELASTICITY: Vector3<f64> = Vector3::new(0.8, 0.2, 0.8);
/// Vanilla `Leashable.SPRING_DAMPENING` (a Java `float`).
pub const SPRING_DAMPENING: f32 = 0.7;
/// Vanilla `Leashable.TORSIONAL_ELASTICITY`.
pub const TORSIONAL_ELASTICITY: f64 = 10.0;
/// Vanilla `Leashable.STIFFNESS`.
pub const STIFFNESS: f64 = 0.11;
/// Vanilla `Leashable.ENTITY_ATTACHMENT_POINT`.
pub const ENTITY_ATTACHMENT_POINT: [Vector3<f64>; 1] = [Vector3::new(0.0, 0.5, 0.5)];
/// Vanilla `Leashable.LEASHER_ATTACHMENT_POINT`.
pub const LEASHER_ATTACHMENT_POINT: [Vector3<f64>; 1] = [Vector3::new(0.0, 0.5, 0.0)];
/// Vanilla `Leashable.SHARED_QUAD_ATTACHMENT_POINTS`.
pub const SHARED_QUAD_ATTACHMENT_POINTS: [Vector3<f64>; 4] = [
    Vector3::new(-0.5, 0.5, 0.5),
    Vector3::new(-0.5, 0.5, -0.5),
    Vector3::new(0.5, 0.5, -0.5),
    Vector3::new(0.5, 0.5, 0.5),
];

/// Vanilla `Leashable.Wrench`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wrench {
    pub force: Vector3<f64>,
    pub torque: f64,
}

impl Wrench {
    pub const ZERO: Self = Self {
        force: Vector3::new(0.0, 0.0, 0.0),
        torque: 0.0,
    };

    /// `leverArm.z * force.x - leverArm.x * force.z`.
    #[must_use]
    pub fn torque_from_force(lever_arm: Vector3<f64>, force: Vector3<f64>) -> f64 {
        lever_arm.z * force.x - lever_arm.x * force.z
    }

    /// Sums x, y, z and torque in list order (`ZERO` for an empty list).
    #[must_use]
    pub fn accumulate(wrenches: &[Self]) -> Self {
        if wrenches.is_empty() {
            return Self::ZERO;
        }
        let (mut x, mut y, mut z, mut t) = (0.0, 0.0, 0.0, 0.0);
        for wrench in wrenches {
            x += wrench.force.x;
            y += wrench.force.y;
            z += wrench.force.z;
            t += wrench.torque;
        }
        Self {
            force: Vector3::new(x, y, z),
            torque: t,
        }
    }

    #[must_use]
    pub fn scale(self, scale: f64) -> Self {
        Self {
            force: self.force * scale,
            torque: self.torque * scale,
        }
    }
}

/// Vanilla `computeDampenedSpringInteraction`.
#[must_use]
pub fn compute_dampened_spring_interaction(
    pivot: Vector3<f64>,
    object: Vector3<f64>,
    slack: f64,
    object_motion: Vector3<f64>,
    lever_arm: Vector3<f64>,
) -> Option<Wrench> {
    // Vec3.distanceTo: d = other - this, sqrt(dx*dx + dy*dy + dz*dz).
    let d = pivot - object;
    let distance = (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
    if distance < slack {
        return None;
    }
    let mut displacement = (pivot - object).normalize() * (distance - slack);
    let torque = Wrench::torque_from_force(lever_arm, displacement);
    if object_motion.dot(&displacement) >= 0.0 {
        displacement = displacement * f64::from(0.3f32);
    }
    Some(Wrench {
        force: displacement,
        torque,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values computed independently in Python (IEEE doubles, f32-rounded 0.3).
    const K: f64 = 0.300_000_011_920_928_96;

    fn v(x: f64, y: f64, z: f64) -> Vector3<f64> {
        Vector3::new(x, y, z)
    }

    #[test]
    fn torque_from_force_sign() {
        // lever (1,0,2), force (3,0,5): 2*3 - 1*5 = 1
        assert_eq!(
            Wrench::torque_from_force(v(1.0, 0.0, 2.0), v(3.0, 0.0, 5.0)),
            1.0
        );
        assert_eq!(
            Wrench::torque_from_force(v(0.0, 0.0, 1.0), v(1.0, 0.0, 0.0)),
            1.0
        );
        assert_eq!(
            Wrench::torque_from_force(v(1.0, 0.0, 0.0), v(0.0, 0.0, 1.0)),
            -1.0
        );
    }

    #[test]
    fn accumulate_sums_in_order() {
        let a = Wrench {
            force: v(1.0, 2.0, 3.0),
            torque: 0.5,
        };
        let b = Wrench {
            force: v(-0.25, 4.0, 1.0),
            torque: 1.5,
        };
        let r = Wrench::accumulate(&[a, b]);
        assert_eq!(
            r,
            Wrench {
                force: v(0.75, 6.0, 4.0),
                torque: 2.0
            }
        );
        assert_eq!(Wrench::accumulate(&[]), Wrench::ZERO);
        assert_eq!(r.scale(0.25).force, v(0.1875, 1.5, 1.0));
        assert_eq!(r.scale(0.25).torque, 0.5);
    }

    #[test]
    fn spring_inside_slack_is_none() {
        let r = compute_dampened_spring_interaction(
            v(5.0, 0.0, 0.0),
            v(0.0, 0.0, 0.0),
            6.0,
            v(0.0, 0.0, 0.0),
            v(0.0, 0.0, 0.5),
        );
        assert_eq!(r, None);
    }

    #[test]
    fn spring_at_slack_has_zero_force() {
        // distance == slack is not "< slack": a zero-length displacement results.
        let r = compute_dampened_spring_interaction(
            v(6.0, 0.0, 0.0),
            v(0.0, 0.0, 0.0),
            6.0,
            v(0.0, 0.0, 0.0),
            v(0.0, 0.0, 0.5),
        )
        .expect("at slack is not None");
        assert_eq!(r.force, v(0.0, 0.0, 0.0));
    }

    #[test]
    fn spring_zero_motion_scales_by_f32_0_3() {
        // 2.0 * f64::from(0.3f32) == 0.6000000238418579
        let r = compute_dampened_spring_interaction(
            v(8.0, 0.0, 0.0),
            v(0.0, 0.0, 0.0),
            6.0,
            v(0.0, 0.0, 0.0),
            v(0.0, 0.0, 1.0),
        )
        .expect("past slack");
        assert_eq!(r.force.x.to_bits(), 0.600_000_023_841_857_9_f64.to_bits());
        assert_eq!(r.force.x.to_bits(), (2.0 * K).to_bits());
        // torque uses the unscaled displacement: 1.0 * 2.0
        assert_eq!(r.torque, 2.0);
    }

    #[test]
    fn spring_opposing_motion_is_unscaled() {
        let r = compute_dampened_spring_interaction(
            v(8.0, 0.0, 0.0),
            v(0.0, 0.0, 0.0),
            6.0,
            v(-1.0, 0.0, 0.0),
            v(0.0, 0.0, 1.0),
        )
        .expect("past slack");
        assert_eq!(r.force, v(2.0, 0.0, 0.0));
        assert_eq!(r.torque, 2.0);
    }

    #[test]
    fn spring_oblique_matches_independent_values() {
        let pivot = v(3.0, 4.0, 5.5);
        let obj = v(0.5, -1.0, 0.25);
        let lever = v(0.5, 0.0, 1.5);
        let r = compute_dampened_spring_interaction(pivot, obj, 6.0, v(0.0, 0.0, 0.0), lever)
            .expect("past slack");
        assert_eq!(r.force.x.to_bits(), 0.163_216_776_510_724_17_f64.to_bits());
        assert_eq!(r.force.y.to_bits(), 0.326_433_553_021_448_33_f64.to_bits());
        assert_eq!(r.force.z.to_bits(), 0.342_755_230_672_520_7_f64.to_bits());
        assert_eq!(r.torque.to_bits(), 0.244_825_155_037_608_6_f64.to_bits());
    }
}
