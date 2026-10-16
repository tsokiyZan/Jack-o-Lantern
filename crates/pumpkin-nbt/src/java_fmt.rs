//! Java's `Double.toString` and `Float.toString`, which vanilla uses whenever it prints a number.

use std::str::FromStr;

/// Formats `value` like Java's `Double.toString` (JDK 19 and later).
#[must_use]
pub fn double_to_string(value: f64) -> String {
    to_java_string(value)
}

/// Formats `value` like Java's `Float.toString` (JDK 19 and later).
#[must_use]
pub fn float_to_string(value: f32) -> String {
    to_java_string(value)
}

trait JavaFloat: Copy + PartialOrd + FromStr + std::fmt::LowerExp {
    fn is_nan(self) -> bool;
    fn is_infinite(self) -> bool;
    fn is_zero(self) -> bool;
    fn is_negative(self) -> bool;
    fn same_value(self, other: Self) -> bool;
    /// More significant digits than the exact decimal expansion of any value of the type has.
    const EXACT_DIGITS: usize;
}

macro_rules! java_float {
    ($ty:ty, $exact_digits:literal) => {
        impl JavaFloat for $ty {
            const EXACT_DIGITS: usize = $exact_digits;
            fn is_nan(self) -> bool {
                <$ty>::is_nan(self)
            }
            fn is_infinite(self) -> bool {
                <$ty>::is_infinite(self)
            }
            fn is_zero(self) -> bool {
                self == 0.0
            }
            fn is_negative(self) -> bool {
                self.is_sign_negative()
            }
            fn same_value(self, other: Self) -> bool {
                self.to_bits() == other.to_bits()
            }
        }
    };
}

java_float!(f32, 120);
java_float!(f64, 770);

/// Splits Rust's `{:e}` output into its decimal digits and the exponent of the first digit.
fn split_exp(formatted: &str) -> (Vec<u8>, i32) {
    let (mantissa, exponent) = formatted.split_once('e').unwrap_or((formatted, "0"));
    let digits = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    (digits, exponent.parse().unwrap_or(0))
}

fn parses_to<T: JavaFloat>(digits: &[u8], exponent: i32, value: T) -> bool {
    let mut text = String::with_capacity(digits.len() + 8);
    for &d in digits {
        text.push(char::from(b'0' + d));
    }
    text.push('e');
    text.push_str(&(exponent - digits.len() as i32 + 1).to_string());
    text.parse::<T>()
        .is_ok_and(|parsed| parsed.same_value(value))
}

/// Picks the decimal Java prints from those of `length` digits that round to `value`: the one
/// closest to `value`, or the one with an even last digit on a tie. A shortest length of 1 is
/// passed as 2, because Java then also considers two-digit decimals (it prints 4.9E-324, not
/// 5.0E-324).
fn closest_of_length<T: JavaFloat>(value: T, length: usize) -> (Vec<u8>, i32) {
    let (exact, exponent) = split_exp(&format!("{value:.*e}", T::EXACT_DIGITS));
    let lower = exact[..length].to_vec();
    let mut upper = lower.clone();
    let mut upper_exp = exponent;
    if let Some(i) = upper.iter().rposition(|&d| d != 9) {
        upper[i] += 1;
        upper[i + 1..].fill(0);
    } else {
        upper.fill(0);
        upper[0] = 1;
        upper_exp += 1;
    }
    let rest = &exact[length..];
    let upper_closer = match rest.first() {
        Some(&d) if d > 5 => true,
        Some(5) if rest[1..].iter().any(|&d| d != 0) => true,
        Some(5) => lower[length - 1] % 2 == 1,
        _ => false,
    };
    let (first, second) = if upper_closer {
        ((upper, upper_exp), (lower, exponent))
    } else {
        ((lower, exponent), (upper, upper_exp))
    };
    if parses_to(&first.0, first.1, value) {
        first
    } else {
        second
    }
}

fn to_java_string<T: JavaFloat>(value: T) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    let negative = value.is_negative();
    if value.is_infinite() {
        return if negative { "-Infinity" } else { "Infinity" }.to_owned();
    }
    if value.is_zero() {
        return if negative { "-0.0" } else { "0.0" }.to_owned();
    }
    let shortest = format!("{value:e}");
    let abs = shortest.trim_start_matches('-');
    let (shortest_digits, _) = split_exp(abs);
    let Ok(abs_value) = abs.parse::<T>() else {
        return shortest;
    };
    let (mut digits, exponent) = closest_of_length(abs_value, shortest_digits.len().max(2));
    while digits.len() > 1 && digits.last() == Some(&0) {
        digits.pop();
    }

    let mut out = String::with_capacity(digits.len() + 8);
    if negative {
        out.push('-');
    }
    let digit = |d: u8| char::from(b'0' + d);
    if (-3..7).contains(&exponent) {
        if exponent < 0 {
            out.push_str("0.");
            for _ in 0..(-exponent - 1) {
                out.push('0');
            }
            out.extend(digits.iter().copied().map(digit));
        } else {
            let int_len = exponent as usize + 1;
            for i in 0..int_len {
                out.push(digits.get(i).copied().map_or('0', digit));
            }
            out.push('.');
            if digits.len() > int_len {
                out.extend(digits[int_len..].iter().copied().map(digit));
            } else {
                out.push('0');
            }
        }
    } else {
        out.push(digit(digits[0]));
        out.push('.');
        if digits.len() > 1 {
            out.extend(digits[1..].iter().copied().map(digit));
        } else {
            out.push('0');
        }
        out.push('E');
        out.push_str(&exponent.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{double_to_string, float_to_string};

    // Expected strings come from Java 25's Double.toString and Float.toString.
    #[test]
    fn matches_java() {
        for (bits, java) in [
            (0x3ff0_0000_0000_0000, "1.0"),
            (0x3fb9_9999_9999_999a, "0.1"),
            (0x3f1a_36e2_eb1c_432d, "1.0E-4"),
            (0x4415_af1d_78b5_8c40, "1.0E20"),
            (0x4089_bd04_a000_0000, "823.6272583007812"),
            (0x0000_0000_0000_0001, "4.9E-324"),
            (0x7fef_ffff_ffff_ffff, "1.7976931348623157E308"),
        ] {
            assert_eq!(double_to_string(f64::from_bits(bits)), java);
        }
        for (bits, java) in [
            (0x3dcc_cccd, "0.1"),
            (0x4a77_2759, "4049366.2"),
            (0x0000_0001, "1.4E-45"),
            (0x7f7f_ffff, "3.4028235E38"),
        ] {
            assert_eq!(float_to_string(f32::from_bits(bits)), java);
        }
        assert_eq!(double_to_string(1e7), "1.0E7");
        assert_eq!(double_to_string(9_999_999.0), "9999999.0");
        assert_eq!(double_to_string(0.001), "0.001");
        assert_eq!(double_to_string(-0.0), "-0.0");
    }
}
