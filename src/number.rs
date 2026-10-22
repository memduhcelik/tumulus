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

/// Sentinels for values whose printed form is not enough to tell them apart
/// from a finite number or from `null`.
const POS_INF: &str = "\u{0}+";
const NEG_INF: &str = "\u{0}-";
const NAN_TEXT: &str = "\u{0}n";
const NEG_NAN_TEXT: &str = "\u{0}N";

pub fn pos_inf() -> String {
    POS_INF.to_string()
}

pub fn neg_inf() -> String {
    NEG_INF.to_string()
}

pub fn nan_text() -> String {
    NAN_TEXT.to_string()
}

pub fn neg_nan_text() -> String {
    NEG_NAN_TEXT.to_string()
}

fn is_nan_body(text: &str) -> bool {
    text == NAN_TEXT || text == NEG_NAN_TEXT
}

pub fn write_number(out: &mut String, text: &str) {
    let text = body(text);
    if text == POS_INF {
        out.push_str(DBL_MAX);
    } else if text == NEG_INF {
        out.push('-');
        out.push_str(DBL_MAX);
    } else if is_nan_body(text) {
        out.push_str("null");
    } else {
        out.push_str(text);
    }
}

/// A number as arithmetic sees it. `Nan` is not equal to itself.
pub enum Real {
    Nan,
    Num(f64),
}

/// jq's conversion before `%`. Values outside `[-2^63, 2^63]` saturate.
/// `2^63` is the one value in that window that does not fit in `i64`.
/// The C cast of it is the minimum on x86_64 and the maximum elsewhere.
pub fn jq_dtoi(n: f64) -> i64 {
    let min = i64::MIN as f64;
    if n < min {
        return i64::MIN;
    }
    if -n < min {
        return i64::MAX;
    }
    if n == i64::MAX as f64 {
        return if cfg!(any(target_arch = "x86", target_arch = "x86_64")) {
            i64::MIN
        } else {
            i64::MAX
        };
    }
    n as i64
}

pub fn real(text: &str) -> Real {
    let text = body(text);
    if is_nan_body(text) {
        return Real::Nan;
    }
    if text == POS_INF {
        return Real::Num(f64::INFINITY);
    }
    if text == NEG_INF {
        return Real::Num(f64::NEG_INFINITY);
    }
    match text.parse::<f64>() {
        Ok(n) if n.is_nan() => Real::Nan,
        Ok(n) => Real::Num(n),
        Err(_) => Real::Nan,
    }
}

/// The binary64 value, including the sign of a NaN or a zero.
/// `real` folds both NaN signs together because they compare the same.
pub fn f64_of(text: &str) -> f64 {
    let text = body(text);
    if text == NEG_NAN_TEXT {
        return -f64::NAN;
    }
    if text == NAN_TEXT {
        return f64::NAN;
    }
    if text == POS_INF {
        return f64::INFINITY;
    }
    if text == NEG_INF {
        return f64::NEG_INFINITY;
    }
    text.parse::<f64>().unwrap_or(f64::NAN)
}

/// x86 `cvttsd2si` yields the indefinite integer for NaN and for a value
/// that does not fit. The other architectures saturate, and NaN becomes 0.
pub(crate) fn c_cast_i64(n: f64) -> i64 {
    if cfg!(any(target_arch = "x86", target_arch = "x86_64")) && out_of_i64(n) {
        i64::MIN
    } else {
        n as i64
    }
}

pub(crate) fn c_cast_i32(n: f64) -> i32 {
    if cfg!(any(target_arch = "x86", target_arch = "x86_64")) && out_of_i32(n) {
        i32::MIN
    } else {
        n as i32
    }
}

fn out_of_i64(n: f64) -> bool {
    !n.is_finite() || n >= i64::MAX as f64 || n < i64::MIN as f64
}

fn out_of_i32(n: f64) -> bool {
    !n.is_finite() || n >= i32::MAX as f64 + 1.0 || n < i32::MIN as f64
}

/// An arithmetic result. Its printed text is not the preserved literal.
pub fn mark_f64(text: String) -> String {
    let mut out = String::with_capacity(text.len() + 1);
    out.push('\u{1}');
    out.push_str(&text);
    out
}

fn body(text: &str) -> &str {
    text.strip_prefix('\u{1}').unwrap_or(text)
}

fn is_marked(text: &str) -> bool {
    text.starts_with('\u{1}')
}

/// Literals keep their decimal. A value that has been through arithmetic
/// compares as a binary64 number, and two literals compare as decimals.
pub fn cmp_number_text(left: &str, right: &str) -> i8 {
    let left_body = body(left);
    let right_body = body(right);
    let left_exact = !is_marked(left) && is_finite_decimal(left_body);
    let right_exact = !is_marked(right) && is_finite_decimal(right_body);
    if left_exact && right_exact {
        return cmp_decimal(left_body, right_body);
    }
    cmp_real(real(left), real(right))
}

fn is_finite_decimal(text: &str) -> bool {
    !matches!(text, POS_INF | NEG_INF | NAN_TEXT | NEG_NAN_TEXT)
}

fn cmp_real(left: Real, right: Real) -> i8 {
    match (left, right) {
        (Real::Nan, _) => -1,
        (_, Real::Nan) => 1,
        (Real::Num(a), Real::Num(b)) => {
            if a < b {
                -1
            } else if a > b {
                1
            } else {
                0
            }
        }
    }
}

fn cmp_decimal(left: &str, right: &str) -> i8 {
    let Some(left_num) = scan_decimal(left) else {
        return cmp_real(real(left), real(right));
    };
    let Some(right_num) = scan_decimal(right) else {
        return cmp_real(real(left), real(right));
    };
    match (left_num.sig(), right_num.sig()) {
        (None, None) => 0,
        (None, Some(_)) => {
            if right_num.negative {
                1
            } else {
                -1
            }
        }
        (Some(_), None) => {
            if left_num.negative {
                -1
            } else {
                1
            }
        }
        (Some(left_sig), Some(right_sig)) => {
            let mag = cmp_sig(&left_num, left_sig, &right_num, right_sig);
            if left_num.negative == right_num.negative {
                if left_num.negative {
                    -mag
                } else {
                    mag
                }
            } else if left_num.negative {
                -1
            } else {
                1
            }
        }
    }
}

/// A literal split into the slices `decimal_parts` would copy.
struct Scanned<'a> {
    negative: bool,
    int_digits: &'a [u8],
    frac_digits: &'a [u8],
    /// `None` is a scale that does not fit in `i32`.
    scale: Option<i32>,
    /// Huge positive exponent: magnitude `1 × 10^10000`.
    huge: bool,
}

/// First and last significant digit, as indexes into integer-then-fraction.
#[derive(Clone, Copy)]
struct Sig {
    first: usize,
    last: usize,
}

impl Scanned<'_> {
    fn total(&self) -> usize {
        self.int_digits.len() + self.frac_digits.len()
    }

    fn digit(&self, index: usize) -> u8 {
        if index < self.int_digits.len() {
            self.int_digits[index]
        } else {
            self.frac_digits[index - self.int_digits.len()]
        }
    }

    /// `None` is zero, including a coefficient of only zeros and a huge
    /// negative exponent.
    fn sig(&self) -> Option<Sig> {
        if self.huge {
            return Some(Sig { first: 0, last: 0 });
        }
        self.scale?;
        let total = self.total();
        let mut first = 0;
        while first < total && self.digit(first) == b'0' {
            first += 1;
        }
        if first == total {
            return None;
        }
        let mut last = total - 1;
        while last > first && self.digit(last) == b'0' {
            last -= 1;
        }
        Some(Sig { first, last })
    }

    /// Power of ten of the first significant digit.
    fn leading_exp(&self, sig: Sig) -> i64 {
        if self.huge {
            return 10_000;
        }
        let scale = i64::from(self.scale.unwrap_or(0));
        scale + self.total() as i64 - sig.first as i64 - 1
    }
}

fn cmp_sig(left: &Scanned<'_>, left_sig: Sig, right: &Scanned<'_>, right_sig: Sig) -> i8 {
    let left_exp = left.leading_exp(left_sig);
    let right_exp = right.leading_exp(right_sig);
    if left_exp != right_exp {
        return if left_exp < right_exp { -1 } else { 1 };
    }
    let left_len = left_sig.last - left_sig.first + 1;
    let right_len = right_sig.last - right_sig.first + 1;
    let shared = left_len.min(right_len);
    for offset in 0..shared {
        let a = left.digit(left_sig.first + offset);
        let b = right.digit(right_sig.first + offset);
        if a != b {
            return if a < b { -1 } else { 1 };
        }
    }
    ord_len(left_len, right_len)
}

fn ord_len(left: usize, right: usize) -> i8 {
    match left.cmp(&right) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// The same split as `decimal_parts`, without copying the digits.
fn scan_decimal(literal: &str) -> Option<Scanned<'_>> {
    let bytes = literal.as_bytes();
    let mut index = 0;
    let negative = if bytes.first() == Some(&b'-') {
        index = 1;
        true
    } else {
        false
    };
    let int_at = index;
    let (int_end, frac_at, frac_end) = if bytes.get(index) == Some(&b'.') {
        index += 1;
        let frac_at = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        (int_at, frac_at, index)
    } else {
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        let int_end = index;
        if bytes.get(index) == Some(&b'.') {
            index += 1;
            let frac_at = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            (int_end, frac_at, index)
        } else {
            (int_end, index, index)
        }
    };
    let fraction = frac_end - frac_at;
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
        let exp_digits = &literal[start..index];
        let trimmed = exp_digits.trim_start_matches('0');
        if trimmed.len() > 9 {
            return Some(Scanned {
                negative,
                int_digits: if exp_negative { &[] } else { b"1" },
                frac_digits: &[],
                scale: if exp_negative { None } else { Some(10_000) },
                huge: !exp_negative,
            });
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
    let scale = exponent.checked_sub(fraction as i32)?;
    Some(Scanned {
        negative,
        int_digits: &bytes[int_at..int_end],
        frac_digits: &bytes[frac_at..frac_end],
        scale: Some(scale),
        huge: false,
    })
}

/// Digits without leading or trailing zeros, and the power of ten of the
/// last digit. `None` is zero.
#[cfg(test)]
fn canonical_mag(digits: String, scale: i32) -> Option<(String, i64)> {
    let lead = digits.chars().take_while(|ch| *ch == '0').count();
    if lead == digits.len() {
        return None;
    }
    let mut digits = if lead > 0 {
        digits[lead..].to_string()
    } else {
        digits
    };
    let mut scale = i64::from(scale);
    while digits.ends_with('0') {
        digits.pop();
        scale += 1;
    }
    Some((digits, scale))
}

#[cfg(test)]
fn cmp_mag(left: &str, left_scale: i64, right: &str, right_scale: i64) -> i8 {
    let left_exp = left_scale + left.len() as i64 - 1;
    let right_exp = right_scale + right.len() as i64 - 1;
    if left_exp != right_exp {
        return if left_exp < right_exp { -1 } else { 1 };
    }
    match left.cmp(right) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// Print a computed number the way jq prints an arithmetic result.
/// Literals keep `normalize`; this is only for results of operators.
/// Two shortest decimals that are equally close keep the even last digit.
pub fn format_jq_f64(n: f64) -> String {
    if n.is_nan() {
        return if n.is_sign_negative() {
            neg_nan_text()
        } else {
            nan_text()
        };
    }
    if n.is_infinite() {
        return if n.is_sign_negative() {
            neg_inf()
        } else {
            pos_inf()
        };
    }
    if n == 0.0 {
        return if n.is_sign_negative() {
            "-0".to_string()
        } else {
            "0".to_string()
        };
    }
    let negative = n.is_sign_negative();
    let (digits, decpt) = decompose(n.abs());
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(&format_digits(&digits, decpt));
    out
}

/// `digits` is the shortest coefficient and `decpt` places the point so the
/// value is `0.digits * 10^decpt`. An exact tie keeps the even last digit.
fn decompose(abs: f64) -> (String, i32) {
    let (digits, decpt) = rust_decimal(abs);
    if let Some((next, next_pt)) = even_halfway(&digits, decpt, abs) {
        (next, next_pt)
    } else {
        (digits, decpt)
    }
}

/// Rust's shortest decimal. On an exact tie it keeps the candidate farther
/// from zero, so the last digit can be the odd one.
fn rust_decimal(abs: f64) -> (String, i32) {
    let text = format!("{abs}");
    let exp_at = text.find(['e', 'E']);
    let (mant, exp) = match exp_at {
        Some(at) => (&text[..at], text[at + 1..].parse::<i32>().unwrap_or(0)),
        None => (text.as_str(), 0),
    };
    let mut digits = String::new();
    let mut point: Option<i32> = None;
    for ch in mant.chars() {
        if ch == '.' {
            point = Some(digits.len() as i32);
            continue;
        }
        if ch.is_ascii_digit() {
            digits.push(ch);
        }
    }
    let mut point = point.unwrap_or(digits.len() as i32);
    let lead = digits.chars().take_while(|ch| *ch == '0').count() as i32;
    if lead as usize >= digits.len() {
        return ("0".to_string(), 0);
    }
    if lead > 0 {
        digits = digits[lead as usize..].to_string();
        point -= lead;
    }
    while digits.ends_with('0') {
        digits.pop();
    }
    if digits.is_empty() {
        return ("0".to_string(), 0);
    }
    (digits, point + exp)
}

/// The other shortest decimal, when `abs` is exactly halfway and Rust kept
/// the odd last digit. Nothing is changed when that other decimal does not
/// parse back to `abs`.
fn even_halfway(digits: &str, decpt: i32, abs: f64) -> Option<(String, i32)> {
    let dir = tie_dir(abs, digits, decpt)?;
    let m: u128 = digits.parse().ok()?;
    if m.is_multiple_of(2) {
        return None;
    }
    let other = if dir > 0 {
        m.checked_add(1)?
    } else {
        m.checked_sub(1)?
    };
    let (next, next_pt) = rescale(other, decpt, digits.len())?;
    let rendered = format_digits(&next, next_pt);
    if rendered.parse::<f64>() == Ok(abs) {
        Some((next, next_pt))
    } else {
        None
    }
}

/// `Some(1)` when `abs` is halfway between this decimal and the next one up.
/// `Some(-1)` when it is halfway between this decimal and the next one down.
///
/// `abs` is an odd integer times a power of two. That power matches the last
/// digit's place on a decimal midpoint, and the odd integer is `2 * digits ± 1`
/// times the fives in the matching power of ten.
fn tie_dir(abs: f64, digits: &str, decpt: i32) -> Option<i32> {
    let m: u128 = digits.parse().ok()?;
    if m == 0 {
        return None;
    }
    let e = decpt - digits.len() as i32;
    let (sig, exp) = odd_parts(abs);
    if exp + 1 != e || sig == 0 {
        return None;
    }
    let q = if e >= 0 {
        let p5 = pow5(u32::try_from(e).ok()?)?;
        let sig = u128::from(sig);
        if sig % p5 != 0 {
            return None;
        }
        sig / p5
    } else {
        let f = u32::try_from(e.checked_neg()?).ok()?;
        let p5 = pow5(f)?;
        u128::from(sig).checked_mul(p5)?
    };
    let twice = m.checked_mul(2)?;
    if q == twice.checked_add(1)? {
        Some(1)
    } else if q.checked_add(1)? == twice {
        Some(-1)
    } else {
        None
    }
}

/// `abs` as `significand * 2^exp` with `significand` odd.
fn odd_parts(n: f64) -> (u64, i32) {
    let bits = n.to_bits();
    let raw_exp = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    let (mut sig, mut exp) = if raw_exp == 0 {
        (frac, -1074)
    } else {
        (frac | (1u64 << 52), raw_exp - 1023 - 52)
    };
    if sig == 0 {
        return (0, 0);
    }
    while sig.is_multiple_of(2) {
        sig /= 2;
        exp += 1;
    }
    (sig, exp)
}

fn pow5(exp: u32) -> Option<u128> {
    let mut acc = 1u128;
    for _ in 0..exp {
        acc = acc.checked_mul(5)?;
    }
    Some(acc)
}

/// `other` is the significand at the same scale as a coefficient of `old_len`
/// digits. Trailing zeros drop out, and a carry into a new digit moves the point.
fn rescale(other: u128, decpt: i32, old_len: usize) -> Option<(String, i32)> {
    if other == 0 {
        return None;
    }
    let mut sig = other;
    let mut zeros = 0i32;
    while sig.is_multiple_of(10) {
        sig /= 10;
        zeros += 1;
    }
    let digits = sig.to_string();
    let new_decpt = decpt - old_len as i32 + zeros + digits.len() as i32;
    Some((digits, new_decpt))
}

fn format_digits(digits: &str, decpt: i32) -> String {
    let len = digits.len() as i32;
    if decpt <= -4 || decpt > len + 15 {
        let mut out = String::new();
        out.push(digits.as_bytes()[0] as char);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        let exp = decpt - 1;
        out.push('e');
        if exp < 0 {
            out.push('-');
        } else {
            out.push('+');
        }
        let body = exp.abs().to_string();
        if body.len() < 2 {
            out.push('0');
        }
        out.push_str(&body);
        return out;
    }
    if decpt <= 0 {
        let mut out = String::from("0.");
        out.extend(std::iter::repeat_n('0', (-decpt) as usize));
        out.push_str(digits);
        return out;
    }
    if decpt >= len {
        let mut out = digits.to_string();
        out.extend(std::iter::repeat_n('0', (decpt - len) as usize));
        return out;
    }
    let at = decpt as usize;
    let mut out = String::new();
    out.push_str(&digits[..at]);
    out.push('.');
    out.push_str(&digits[at..]);
    out
}

/// Truncate a filter index literal toward zero.
/// `None` means the magnitude does not fit in an array index.
pub fn array_index(literal: &str) -> Option<i64> {
    let literal = body(literal);
    if matches!(literal, POS_INF | NEG_INF | NAN_TEXT | NEG_NAN_TEXT) {
        return None;
    }
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
/// `nan` is the start on the left of `:` and the length on the right.
/// `infinite` is the length, and `-infinite` is the start.
pub fn slice_bound(literal: &str, len: i64, end: bool) -> i64 {
    let text = body(literal);
    if text == NAN_TEXT || text == NEG_NAN_TEXT {
        return if end { len } else { 0 };
    }
    if text == POS_INF {
        return len;
    }
    if text == NEG_INF {
        return 0;
    }
    // A computed bound carries a mark in front of the digits. The mark is not
    // part of the number.
    let Some((negative, digits, scale)) = decimal_parts(text) else {
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
        neg_inf()
    } else {
        pos_inf()
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
    } else if sign.is_empty() {
        pos_inf()
    } else {
        neg_inf()
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

#[cfg(test)]
mod tests {
    use super::{canonical_mag, cmp_decimal, cmp_real, decimal_parts, real};

    fn cmp_alloc(left: &str, right: &str) -> i8 {
        let Some((left_neg, left_digits, left_scale)) = decimal_parts(left) else {
            return cmp_real(real(left), real(right));
        };
        let Some((right_neg, right_digits, right_scale)) = decimal_parts(right) else {
            return cmp_real(real(left), real(right));
        };
        let left_mag = canonical_mag(left_digits, left_scale);
        let right_mag = canonical_mag(right_digits, right_scale);
        match (left_mag, right_mag) {
            (None, None) => 0,
            (None, Some(_)) => {
                if right_neg {
                    1
                } else {
                    -1
                }
            }
            (Some(_), None) => {
                if left_neg {
                    -1
                } else {
                    1
                }
            }
            (Some((ld, ls)), Some((rd, rs))) => {
                let mag = super::cmp_mag(&ld, ls, &rd, rs);
                if left_neg == right_neg {
                    if left_neg {
                        -mag
                    } else {
                        mag
                    }
                } else if left_neg {
                    -1
                } else {
                    1
                }
            }
        }
    }

    #[test]
    fn decimal_scan_matches_the_copying_compare() {
        let samples = [
            "0",
            "0.0",
            "-0",
            "0.00",
            "1",
            "1.0",
            "1.00",
            "1.50",
            "01.20",
            "4.404",
            "12.5",
            ".5",
            "0.04",
            "0.040",
            "10",
            "10.0",
            "100",
            "-4.5",
            "-0.5",
            "1e2",
            "1e+2",
            "1E-1",
            "1e10",
            "1e-10",
            "2.5e1",
            "25",
            "250e-1",
            "1e",
            "e10",
            "99.99",
            "100.00",
            "-1.5e2",
            "-150",
            "1e9",
            "1e10",
            "999999999",
            "1e-9",
            "0001",
            "1.2.3",
            "1e000",
            "1e+0008",
            "-0.0",
            "1e10000000000",
            "1e-10000000000",
            "-2e10000000000",
        ];
        for left in samples {
            for right in samples {
                assert_eq!(
                    cmp_decimal(left, right),
                    cmp_alloc(left, right),
                    "{left} compared with {right}"
                );
            }
        }
    }
}
