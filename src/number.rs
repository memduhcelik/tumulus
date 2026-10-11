//! jq's number text.
//!
//! Leading zeros, a leading `+`, a leading dot, and a trailing dot are
//! rewritten. An exponent is applied to the digits, and trailing zeros in
//! the coefficient stay. Scientific form uses `E` and a signed exponent.
//! The scientific exponent stays inside jq's formatter window; outside it,
//! jq saturates.

const MAX_SCI: i128 = 999_999_999;
const MIN_LSD: i128 = -1_147_483_646;
const DBL_MAX: &str = "1.7976931348623157e+308";

/// jq prints infinities as the maximum finite binary64 value.
/// Truncate a filter index literal toward zero.
/// `None` means the magnitude does not fit in an array index.
pub fn array_index(literal: &str) -> Option<i64> {
    let bytes = literal.as_bytes();
    let mut index = 0;
    let negative = if bytes.first() == Some(&b'-') {
        index = 1;
        true
    } else {
        false
    };
    let mut int_digits = String::new();
    let mut frac_digits = String::new();
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            frac_digits.push(bytes[index] as char);
            index += 1;
        }
    } else {
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            int_digits.push(bytes[index] as char);
            index += 1;
        }
        if bytes.get(index) == Some(&b'.') {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                frac_digits.push(bytes[index] as char);
                index += 1;
            }
        }
    }
    let exponent = if index < bytes.len() && (bytes[index] == b'e' || bytes[index] == b'E') {
        index += 1;
        let exp_negative = if index < bytes.len() && (bytes[index] == b'+' || bytes[index] == b'-')
        {
            let neg = bytes[index] == b'-';
            index += 1;
            neg
        } else {
            false
        };
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        let digits = literal[start..index].trim_start_matches('0');
        if digits.len() > 9 {
            return if exp_negative { Some(0) } else { None };
        }
        let value: i32 = if digits.is_empty() {
            0
        } else {
            digits.parse().unwrap_or(0)
        };
        if exp_negative {
            -value
        } else {
            value
        }
    } else {
        0
    };
    let mut digits = int_digits;
    digits.push_str(&frac_digits);
    if digits.is_empty() {
        return Some(0);
    }
    let point = (literal_int_len(literal)) as i32;
    let Some(new_point) = point.checked_add(exponent) else {
        return if exponent < 0 { Some(0) } else { None };
    };
    if new_point <= 0 {
        return Some(0);
    }
    let int_part = if new_point as usize >= digits.len() {
        let mut widened = digits;
        widened.extend(std::iter::repeat_n('0', new_point as usize - widened.len()));
        widened
    } else {
        digits[..new_point as usize].to_string()
    };
    let trimmed = int_part.trim_start_matches('0');
    if trimmed.is_empty() {
        return Some(0);
    }
    if trimmed.len() > 18 {
        return None;
    }
    let magnitude: i64 = trimmed.parse().ok()?;
    if negative {
        magnitude.checked_neg()
    } else {
        Some(magnitude)
    }
}

/// Bound for a slice. `end` rounds with ceil, the start with floor.
/// A negative value counts back from `len`. The result is clamped to `0..=len`.
pub fn slice_bound(literal: &str, len: i64, end: bool) -> i64 {
    let Some((negative, digits, scale)) = decimal_parts(literal) else {
        return 0;
    };
    // `-0` is zero. A tiny fraction uses `scale == i32::MIN` and must stay signed.
    if scale != i32::MIN && digits.chars().all(|digit| digit == '0') {
        return 0;
    }
    let rounded = if scale == i32::MIN {
        tiny_slice(negative, end)
    } else {
        round_decimal(negative, &digits, scale, end)
    };
    let mut index = rounded;
    if negative {
        index = index.saturating_add(i128::from(len));
    }
    let len = i128::from(len);
    if index < 0 {
        0
    } else if index > len {
        len as i64
    } else {
        index as i64
    }
}

/// Sign, coefficient digits, and the power of ten (`value = ±digits × 10^scale`).
/// `scale == i32::MIN` means a fraction too small to be an integer.
fn decimal_parts(literal: &str) -> Option<(bool, String, i32)> {
    let bytes = literal.as_bytes();
    let mut index = 0;
    let negative = if bytes.first() == Some(&b'-') {
        index = 1;
        true
    } else {
        false
    };
    let mut int_digits = String::new();
    let mut frac_digits = String::new();
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            frac_digits.push(bytes[index] as char);
            index += 1;
        }
    } else {
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            int_digits.push(bytes[index] as char);
            index += 1;
        }
        if bytes.get(index) == Some(&b'.') {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                frac_digits.push(bytes[index] as char);
                index += 1;
            }
        }
    }
    let exponent = if index < bytes.len() && (bytes[index] == b'e' || bytes[index] == b'E') {
        index += 1;
        let exp_negative = if index < bytes.len() && (bytes[index] == b'+' || bytes[index] == b'-')
        {
            let neg = bytes[index] == b'-';
            index += 1;
            neg
        } else {
            false
        };
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        let digits = &literal[start..index];
        let trimmed = digits.trim_start_matches('0');
        if trimmed.len() > 9 {
            return if exp_negative {
                Some((negative, String::new(), i32::MIN))
            } else if negative {
                Some((true, "1".to_string(), 10_000))
            } else {
                Some((false, "1".to_string(), 10_000))
            };
        }
        let value: i32 = if trimmed.is_empty() {
            0
        } else {
            trimmed.parse().unwrap_or(0)
        };
        if exp_negative {
            -value
        } else {
            value
        }
    } else {
        0
    };
    let mut digits = int_digits;
    digits.push_str(&frac_digits);
    let scale = exponent.checked_sub(frac_digits.len() as i32)?;
    Some((negative, digits, scale))
}

fn tiny_slice(negative: bool, end: bool) -> i128 {
    if !negative {
        if end {
            1
        } else {
            0
        }
    } else if end {
        0
    } else {
        -1
    }
}

fn round_decimal(negative: bool, digits: &str, scale: i32, end: bool) -> i128 {
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return 0;
    }
    if scale >= 0 {
        if digits.len() as i32 + scale > 38 {
            return if negative { i128::MIN } else { i128::MAX };
        }
        let mut text = digits.to_string();
        text.extend(std::iter::repeat_n('0', scale as usize));
        let magnitude: i128 = text.parse().unwrap_or(i128::MAX);
        return if negative { -magnitude } else { magnitude };
    }
    let (quotient, remainder) = split_decimal(digits, -scale);
    let quotient = parse_sat(&quotient);
    if !negative {
        if end && remainder {
            quotient.saturating_add(1)
        } else {
            quotient
        }
    } else if end || !remainder {
        -quotient
    } else {
        quotient.saturating_add(1).saturating_neg()
    }
}

fn split_decimal(digits: &str, places: i32) -> (String, bool) {
    if places as usize >= digits.len() {
        ("0".to_string(), digits.chars().any(|c| c != '0'))
    } else {
        let cut = digits.len() - places as usize;
        let remainder = digits[cut..].chars().any(|c| c != '0');
        (digits[..cut].to_string(), remainder)
    }
}

fn parse_sat(digits: &str) -> i128 {
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return 0;
    }
    if digits.len() > 38 {
        return i128::MAX;
    }
    digits.parse().unwrap_or(i128::MAX)
}

fn literal_int_len(literal: &str) -> usize {
    let bytes = literal.as_bytes();
    let mut index = usize::from(bytes.first() == Some(&b'-'));
    if bytes.get(index) == Some(&b'.') {
        return 0;
    }
    let start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    index - start
}

pub fn infinite(negative: bool) -> String {
    if negative {
        format!("-{DBL_MAX}")
    } else {
        DBL_MAX.to_string()
    }
}

enum Exp {
    Value(i128),
    PosHuge,
    NegHuge,
}

enum LowFit {
    Ready(String, i128),
    Zero,
}

pub fn normalize(
    negative: bool,
    int_digits: &str,
    frac_digits: Option<&str>,
    exponent: Option<(bool, &str)>,
) -> String {
    let frac = frac_digits.unwrap_or("");
    let mut coef = String::with_capacity(int_digits.len() + frac.len());
    coef.push_str(int_digits);
    coef.push_str(frac);
    let trimmed = coef.trim_start_matches('0');
    let digits = if trimmed.is_empty() { "0" } else { trimmed };
    let exp = match exponent {
        None => Exp::Value(0),
        Some((exp_negative, raw)) => parse_exp(exp_negative, raw),
    };
    let lsd = match exp {
        Exp::PosHuge => Exp::PosHuge,
        Exp::NegHuge => Exp::NegHuge,
        Exp::Value(value) => match value.checked_sub(frac.len() as i128) {
            Some(next) => Exp::Value(next),
            None => Exp::NegHuge,
        },
    };
    render(negative, digits, lsd)
}

fn parse_exp(negative: bool, digits: &str) -> Exp {
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Exp::Value(0);
    }
    if digits.len() > 38 {
        return if negative { Exp::NegHuge } else { Exp::PosHuge };
    }
    let value: i128 = digits.parse().expect("exponent digits fit in range");
    Exp::Value(if negative { -value } else { value })
}

fn render(negative: bool, digits: &str, lsd: Exp) -> String {
    let sign = if negative { "-" } else { "" };
    match lsd {
        Exp::PosHuge => saturate_high(sign, digits == "0"),
        Exp::NegHuge => format!("{sign}0E{MIN_LSD}"),
        Exp::Value(exp) if exp < MIN_LSD => match fit_low(digits, exp) {
            LowFit::Zero => format!("{sign}0E{MIN_LSD}"),
            LowFit::Ready(digits, exp) => scientific(sign, &digits, exp),
        },
        Exp::Value(exp) => {
            let len = digits.len() as i128;
            let below_fixed = len + exp <= -6;
            if exp > 0 || below_fixed {
                let printed = exp + len - 1;
                if printed > MAX_SCI {
                    return saturate_high(sign, digits == "0");
                }
                return scientific(sign, digits, exp);
            }
            fixed(sign, digits, exp)
        }
    }
}

fn saturate_high(sign: &str, zero: bool) -> String {
    if zero {
        format!("{sign}0E+{MAX_SCI}")
    } else {
        format!("{sign}{DBL_MAX}")
    }
}

/// Drop digits that would sit below jq's exponent floor, rounding the
/// magnitude half up.
fn fit_low(digits: &str, lsd: i128) -> LowFit {
    let shift = match MIN_LSD.checked_sub(lsd) {
        Some(shift) => shift,
        None => return LowFit::Zero,
    };
    if shift <= 0 {
        return LowFit::Ready(digits.to_string(), lsd);
    }
    if shift > digits.len() as i128 {
        return LowFit::Zero;
    }
    let shift = shift as usize;
    if shift == digits.len() {
        if digits.as_bytes()[0] >= b'5' {
            return LowFit::Ready("1".to_string(), MIN_LSD);
        }
        return LowFit::Zero;
    }
    let keep = digits.len() - shift;
    let round_up = digits.as_bytes()[keep] >= b'5';
    if !round_up {
        return LowFit::Ready(digits[..keep].to_string(), MIN_LSD);
    }
    match increment(&digits[..keep]) {
        Some(next) => LowFit::Ready(next, MIN_LSD),
        None => {
            let mut next = String::from("1");
            next.extend(std::iter::repeat_n('0', keep));
            LowFit::Ready(next, MIN_LSD)
        }
    }
}

fn increment(digits: &str) -> Option<String> {
    let mut bytes = digits.as_bytes().to_vec();
    for slot in bytes.iter_mut().rev() {
        if *slot < b'9' {
            *slot += 1;
            return Some(String::from_utf8(bytes).expect("digits are ascii"));
        }
        *slot = b'0';
    }
    None
}

fn scientific(sign: &str, digits: &str, lsd: i128) -> String {
    let mut out = String::new();
    out.push_str(sign);
    let mut chars = digits.chars();
    out.push(chars.next().expect("number has a digit"));
    if digits.len() > 1 {
        out.push('.');
        out.push_str(&digits[1..]);
    }
    let printed = lsd + digits.len() as i128 - 1;
    out.push('E');
    if printed < 0 {
        out.push('-');
    } else {
        out.push('+');
    }
    out.push_str(&printed.abs().to_string());
    out
}

fn fixed(sign: &str, digits: &str, lsd: i128) -> String {
    let len = digits.len() as i128;
    let left = len + lsd;
    let mut out = String::new();
    out.push_str(sign);
    if left <= 0 {
        out.push_str("0.");
        if left < 0 {
            out.extend(std::iter::repeat_n('0', (-left) as usize));
        }
        out.push_str(digits);
        return out;
    }
    if left == len {
        out.push_str(digits);
        return out;
    }
    let left = left as usize;
    out.push_str(&digits[..left]);
    out.push('.');
    out.push_str(&digits[left..]);
    out
}
