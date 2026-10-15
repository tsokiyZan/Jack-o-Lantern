use pumpkin_util::math::vector3::Vector3;

use crate::{
    VarInt,
    ser::{NetworkWriteExt, ReadingError, WritingError},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LpVector3d(pub Vector3<f64>);

impl LpVector3d {
    pub fn write<W: std::io::Write>(&self, writer: &mut W) -> Result<(), WritingError> {
        let velocity = self.0;

        // Clamp and find the maximum component magnitude
        let clamped_x = clamp_value(velocity.x);
        let clamped_y = clamp_value(velocity.y);
        let clamped_z = clamp_value(velocity.z);
        let max_component = abs_max(clamped_x, abs_max(clamped_y, clamped_z));

        if max_component < MIN_VELOCITY_MAGNITUDE {
            return writer.write_slice(&[0u8]);
        }

        let scale_factor = max_component.ceil() as i64;
        let is_extended = scale_factor > 3;

        // The header byte: bits 0-1 are scale, bit 2 is the extension flag
        let header = if is_extended {
            (scale_factor & 3) | 4
        } else {
            scale_factor
        };

        // Pack the 15-bit quantized components into a 64-bit buffer
        // Quantized components: x (bits 3-17), y (bits 18-32), z (bits 33-47)
        let quantized_x = to_long(clamped_x / scale_factor as f64) << 3;
        let quantized_y = to_long(clamped_y / scale_factor as f64) << 18;
        let quantized_z = to_long(clamped_z / scale_factor as f64) << 33;

        let packed_data: i64 = header | quantized_x | quantized_y | quantized_z;

        // Write low 16 bits (Little Endian)
        writer
            .write_all(&(packed_data as u16).to_le_bytes())
            .map_err(WritingError::IoError)?;

        // Write next 32 bits (Big Endian)
        writer
            .write_all(&((packed_data >> 16) as i32).to_be_bytes())
            .map_err(WritingError::IoError)?;

        if is_extended {
            let scale_tail = VarInt((scale_factor >> 2) as i32);
            writer.write_var_int(&scale_tail)?;
        }

        Ok(())
    }

    /// Vanilla `LpVec3.read`: a zero lowest byte is the whole zero vector.
    pub fn read<R: std::io::Read>(reader: &mut R) -> Result<Self, ReadingError> {
        let mut lowest = [0u8; 1];
        reader
            .read_exact(&mut lowest)
            .map_err(|e| ReadingError::Message(e.to_string()))?;
        if lowest[0] == 0 {
            return Ok(Self(Vector3::new(0.0, 0.0, 0.0)));
        }

        let mut middle_highest = [0u8; 5];
        reader
            .read_exact(&mut middle_highest)
            .map_err(|e| ReadingError::Message(e.to_string()))?;
        let lowest = i64::from(lowest[0]);
        let middle = i64::from(middle_highest[0]);
        let highest = i64::from(u32::from_be_bytes([
            middle_highest[1],
            middle_highest[2],
            middle_highest[3],
            middle_highest[4],
        ]));
        let buffer = (highest << 16) | (middle << 8) | lowest;

        let mut scale = lowest & 3;
        if (lowest & 4) == 4 {
            scale |= (i64::from(VarInt::decode(reader)?.0) & 0xFFFF_FFFF) << 2;
        }
        let scale = scale as f64;

        Ok(Self(Vector3::new(
            from_long(buffer >> 3) * scale,
            from_long(buffer >> 18) * scale,
            from_long(buffer >> 33) * scale,
        )))
    }

    pub fn write_legacy<W: std::io::Write>(&self, writer: &mut W) -> Result<(), WritingError> {
        writer.write_i16_be(encode_legacy_velocity_component(self.0.x))?;
        writer.write_i16_be(encode_legacy_velocity_component(self.0.y))?;
        writer.write_i16_be(encode_legacy_velocity_component(self.0.z))?;
        Ok(())
    }
}

const MAX_VELOCITY_CLAMP: f64 = 1.717_986_918_3E10;
const LEGACY_COMPONENT_CLAMP: f64 = 3.9;
const LEGACY_COMPONENT_SCALE: f64 = 8000.0;
const MIN_VELOCITY_MAGNITUDE: f64 = 3.051_944_088_384_301E-5;
const MAX_15_BIT_VALUE: f64 = 32766.0;

#[must_use]
pub fn encode_legacy_velocity_component(component: f64) -> i16 {
    // Legacy clients (<= 1.21.8 / protocol 772) encode velocity as clamped component * 8000.
    (component.clamp(-LEGACY_COMPONENT_CLAMP, LEGACY_COMPONENT_CLAMP) * LEGACY_COMPONENT_SCALE)
        as i16
}

fn clamp_value(value: f64) -> f64 {
    if value.is_nan() {
        return 0.0;
    }
    value.clamp(-MAX_VELOCITY_CLAMP, MAX_VELOCITY_CLAMP)
}

const fn abs_max(a: f64, b: f64) -> f64 {
    a.abs().max(b.abs())
}

fn to_long(value: f64) -> i64 {
    // Vanilla `LpVec3.pack`: `Math.round((value * 0.5 + 0.5) * 32766.0)`.
    ((value * 0.5 + 0.5) * MAX_15_BIT_VALUE).round() as i64
}

/// Vanilla `LpVec3.unpack`.
fn from_long(value: i64) -> f64 {
    ((value & 0x7FFF) as f64).min(MAX_15_BIT_VALUE) * 2.0 / MAX_15_BIT_VALUE - 1.0
}

#[cfg(test)]
mod tests {
    use super::{LpVector3d, encode_legacy_velocity_component};
    use pumpkin_util::math::vector3::Vector3;

    #[test]
    fn legacy_component_is_clamped_and_scaled() {
        assert_eq!(encode_legacy_velocity_component(0.5), 4000);
        assert_eq!(encode_legacy_velocity_component(-0.5), -4000);
        assert_eq!(encode_legacy_velocity_component(4.0), 31200);
        assert_eq!(encode_legacy_velocity_component(-4.0), -31200);
    }

    #[test]
    fn write_legacy_writes_three_i16_be_components() -> Result<(), Box<dyn std::error::Error>> {
        let velocity = LpVector3d(Vector3::new(0.5, -0.5, 0.0));
        let mut buf = Vec::new();
        velocity.write_legacy(&mut buf)?;
        assert_eq!(buf, vec![0x0F, 0xA0, 0xF0, 0x60, 0x00, 0x00]);
        Ok(())
    }

    #[test]
    fn zero_vector_reads_one_byte() {
        let bytes = [0u8, 0xAB];
        let mut cursor = std::io::Cursor::new(&bytes[..]);
        let vector = LpVector3d::read(&mut cursor).unwrap();
        assert_eq!(vector.0, Vector3::new(0.0, 0.0, 0.0));
        assert_eq!(cursor.position(), 1);
    }

    #[test]
    fn round_trips_through_vanilla_quantization() {
        for velocity in [
            Vector3::new(0.25, -0.0784, 0.5),
            Vector3::new(5.5, -12.0, 0.001),
        ] {
            let mut bytes = Vec::new();
            LpVector3d(velocity).write(&mut bytes).unwrap();
            let decoded = LpVector3d::read(&mut std::io::Cursor::new(&bytes[..])).unwrap();
            assert!((decoded.0.x - velocity.x).abs() < 1e-3);
            assert!((decoded.0.y - velocity.y).abs() < 1e-3);
            assert!((decoded.0.z - velocity.z).abs() < 1e-3);
        }
    }
}
