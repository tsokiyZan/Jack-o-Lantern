use std::f32::consts::{PI, TAU};

use super::vector3::Vector3;

/// JOML `Quaternionf` (with `Options.FASTMATH` off), for the operations vanilla uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quaternionf {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

/// JOML `Math.sin(float)`.
fn sin(angle: f32) -> f32 {
    f64::from(angle).sin() as f32
}

/// JOML `Math.cosFromSin(float, float)`.
fn cos_from_sin(sin: f32, angle: f32) -> f32 {
    let cos = (1.0 - sin * sin).sqrt();
    let a = angle + PI / 2.0;
    let mut b = a - (a / TAU) as i32 as f32 * TAU;
    if b < 0.0 {
        b += TAU;
    }
    if b >= PI { -cos } else { cos }
}

/// JOML `Math.cosFromSin(double, double)`.
fn cos_from_sin_f64(sin: f64, angle: f64) -> f64 {
    use std::f64::consts::{PI, TAU};
    let cos = (1.0 - sin * sin).sqrt();
    let a = angle + PI / 2.0;
    let mut b = a - f64::from((a / TAU) as i32) * TAU;
    if b < 0.0 {
        b += TAU;
    }
    if b >= PI { -cos } else { cos }
}

impl Quaternionf {
    /// JOML `new Quaternionf().setAngleAxis(double angle, double x, double y, double z)`.
    #[must_use]
    pub fn from_angle_axis(angle: f64, x: f64, y: f64, z: f64) -> Self {
        let s = (angle * 0.5).sin();
        Self {
            x: (x * s) as f32,
            y: (y * s) as f32,
            z: (z * s) as f32,
            w: cos_from_sin_f64(s, angle * 0.5) as f32,
        }
    }

    /// JOML `transform(float, float, float, Vector3f)`, as `Vector3f.rotate` uses it. JOML's
    /// `Math.fma` is a plain multiply and add unless `joml.useMathFma` is set.
    #[must_use]
    pub fn transform_f32(self, v: Vector3<f32>) -> Vector3<f32> {
        let (xx, yy, zz, ww) = (
            self.x * self.x,
            self.y * self.y,
            self.z * self.z,
            self.w * self.w,
        );
        let (xy, xz, yz) = (self.x * self.y, self.x * self.z, self.y * self.z);
        let (xw, zw, yw) = (self.x * self.w, self.z * self.w, self.y * self.w);
        let k = 1.0 / (xx + yy + zz + ww);
        Vector3::new(
            (xx - yy - zz + ww) * k * v.x + (2.0 * (xy - zw) * k * v.y + 2.0 * (xz + yw) * k * v.z),
            2.0 * (xy + zw) * k * v.x + ((yy - xx - zz + ww) * k * v.y + 2.0 * (yz - xw) * k * v.z),
            2.0 * (xz - yw) * k * v.x + (2.0 * (yz + xw) * k * v.y + (zz - xx - yy + ww) * k * v.z),
        )
    }
    /// JOML `new Quaternionf().rotationY(angle)`.
    #[must_use]
    pub fn rotation_y(angle: f32) -> Self {
        let sin = sin(angle * 0.5);
        let cos = cos_from_sin(sin, angle * 0.5);
        Self {
            x: 0.0,
            y: sin,
            z: 0.0,
            w: cos,
        }
    }

    /// JOML `rotateX(angle)`.
    #[must_use]
    pub fn rotate_x(self, angle: f32) -> Self {
        let sin = sin(angle * 0.5);
        let cos = cos_from_sin(sin, angle * 0.5);
        Self {
            x: self.w * sin + self.x * cos,
            y: self.y * cos + self.z * sin,
            z: self.z * cos - self.y * sin,
            w: self.w * cos - self.x * sin,
        }
    }

    /// JOML `transform(double, double, double, Vector3d)`. JOML's `Math.fma` is a plain multiply
    /// and add unless `joml.useMathFma` is set, which vanilla doesn't.
    #[must_use]
    pub fn transform(self, v: Vector3<f64>) -> Vector3<f64> {
        let (xx, yy, zz, ww) = (
            self.x * self.x,
            self.y * self.y,
            self.z * self.z,
            self.w * self.w,
        );
        let (xy, xz, yz) = (self.x * self.y, self.x * self.z, self.y * self.z);
        let (xw, zw, yw) = (self.x * self.w, self.z * self.w, self.y * self.w);
        let k = 1.0 / (xx + yy + zz + ww);
        let f = f64::from;
        Vector3::new(
            f((xx - yy - zz + ww) * k) * v.x
                + (f(2.0 * (xy - zw) * k) * v.y + f(2.0 * (xz + yw) * k) * v.z),
            f(2.0 * (xy + zw) * k) * v.x
                + (f((yy - xx - zz + ww) * k) * v.y + f(2.0 * (yz - xw) * k) * v.z),
            f(2.0 * (xz - yw) * k) * v.x
                + (f(2.0 * (yz + xw) * k) * v.y + f((zz - xx - yy + ww) * k) * v.z),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bits from JOML 1.10.9's default (unfused) `transform(double, double, double, Vector3d)` on
    /// vanilla's look quaternion; a fused multiply-add rounds each of these differently.
    #[test]
    fn look_transform_of_any_vector_is_unfused() {
        let to_radians = (std::f64::consts::PI / 180.0) as f32;
        let cases: [(u32, u32, [u64; 3], [u64; 3]); 3] = [
            (
                0xc1c7_00ec,
                0x4098_2c80,
                [
                    0x4004_c89f_bb01_6058,
                    0x3fff_fa0d_90e7_7674,
                    0xbff0_a888_2502_09f7,
                ],
                [
                    0x4005_e56a_24ad_de7e,
                    0x3ff6_00b9_3834_030f,
                    0xbff9_04f7_92e8_b142,
                ],
            ),
            (
                0xc276_0c52,
                0x431a_b590,
                [
                    0x3ff9_d031_664d_b7b0,
                    0x4005_5d10_09bc_d2e0,
                    0xc004_da17_4d0f_e69c,
                ],
                [
                    0x3fb3_3c41_7f6b_4150,
                    0xbff0_4647_2c52_7770,
                    0x400f_7bc3_5b85_eacc,
                ],
            ),
            (
                0x425f_5dd4,
                0xc267_0fa0,
                [
                    0xbfc0_f603_b222_b9c0,
                    0xbff0_125b_5137_567c,
                    0x3ff2_7ba6_7790_7410,
                ],
                [
                    0xbfcc_d0a1_1bff_06ce,
                    0xbff8_5186_99be_71ab,
                    0x3f8e_1a24_be37_b8a8,
                ],
            ),
        ];
        for (x_rot, y_rot, v, want) in cases {
            let (x_rot, y_rot) = (f32::from_bits(x_rot), f32::from_bits(y_rot));
            let v = Vector3::new(
                f64::from_bits(v[0]),
                f64::from_bits(v[1]),
                f64::from_bits(v[2]),
            );
            let out = Quaternionf::rotation_y(-y_rot * to_radians)
                .rotate_x(x_rot * to_radians)
                .transform(v);
            assert_eq!([out.x.to_bits(), out.y.to_bits(), out.z.to_bits()], want);
        }
    }

    /// Bits from JOML 1.10.9: `new Quaternionf().rotationY(-yRot * TO_RAD).rotateX(xRot * TO_RAD)
    /// .transform(0, 0, 1, new Vector3d())`, as vanilla `getLookQuaternion` uses it.
    #[test]
    fn look_transform_matches_joml() {
        let to_radians = (std::f64::consts::PI / 180.0) as f32;
        let cases: [(f32, f32, [u64; 3]); 4] = [
            (0.0, 0.0, [0, 0, 0x3ff0_0000_0000_0000]),
            (
                30.0,
                45.0,
                [
                    0xbfe3_988e_2000_0000,
                    0xbfdf_ffff_e000_0000,
                    0x3fe3_988e_0000_0000,
                ],
            ),
            (
                -89.9,
                -170.5,
                [
                    0x3f32_e0e0_2000_0000,
                    0x3fef_fffc_e000_0000,
                    0xbf5c_33e7_4000_0000,
                ],
            ),
            (
                12.34,
                1234.5,
                [
                    0xbfda_ea8a_0000_0000,
                    0xbfcb_5aee_0000_0000,
                    0xbfec_3727_0000_0000,
                ],
            ),
        ];
        for (x_rot, y_rot, bits) in cases {
            let v = Quaternionf::rotation_y(-y_rot * to_radians)
                .rotate_x(x_rot * to_radians)
                .transform(Vector3::new(0.0, 0.0, 1.0));
            assert_eq!(
                [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()],
                bits,
                "x_rot {x_rot} y_rot {y_rot}"
            );
        }
    }
}
