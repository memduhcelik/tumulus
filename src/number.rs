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
