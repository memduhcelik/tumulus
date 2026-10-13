//! `todate`, `fromdate`, `gmtime`, `mktime`, and `strftime`.
//!
//! Broken-down times go through the C library, so each platform agrees
//! with the jq it ships. `strftime` accepts `%Y` `%m` `%d` `%H` `%M`
//! `%S` and `%Z`.

use crate::number;
use crate::value::Value;
use std::ffi::c_char;

const ISO: &str = "%Y-%m-%dT%H:%M:%SZ";

/// `struct tm` on macOS and glibc, LP64. `gmtime_r` fills `tm_zone`.
/// jq clears that pointer before `strftime`, so `%Z` is the local zone
/// while the calendar fields stay UTC.
#[repr(C)]
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const c_char,
}

const _: () = {
    assert!(std::mem::size_of::<Tm>() == 56);
    assert!(std::mem::align_of::<Tm>() == 8);
    assert!(std::mem::offset_of!(Tm, tm_year) == 20);
    assert!(std::mem::offset_of!(Tm, tm_zone) == 48);
};

extern "C" {
    fn gmtime_r(clock: *const i64, out: *mut Tm) -> *mut Tm;
    fn timegm(tm: *mut Tm) -> i64;
    fn strftime(buf: *mut c_char, max: usize, format: *const c_char, tm: *const Tm) -> usize;
    fn strptime(text: *const c_char, format: *const c_char, tm: *mut Tm) -> *mut c_char;
    fn isspace(c: i32) -> i32;
}

pub(crate) fn todate(input: &Value) -> Result<Value, String> {
    format_time(input, &Value::String(ISO.to_string()))
}

pub(crate) fn fromdate(input: &Value) -> Result<Value, String> {
    let Value::String(text) = input else {
        return Err("strptime/1 requires string inputs and arguments".to_string());
    };
    let mut raw = text.as_bytes().to_vec();
    raw.push(0);
    let format = b"%Y-%m-%dT%H:%M:%SZ\0";
    let mut tm = blank();
    let end = unsafe {
        strptime(
            raw.as_ptr() as *const c_char,
            format.as_ptr() as *const c_char,
            &mut tm,
        )
    };
    if end.is_null() || !trailing_space(end) {
        // jq prints the text with `%s`, which stops at the first NUL.
        let shown = text.split('\0').next().unwrap_or("");
        return Err(format!("date \"{shown}\" does not match format \"{ISO}\""));
    }
    // `timegm` ignores the weekday and yearday `strptime` stored.
    stamp(&mut tm)
}

pub(crate) fn gmtime(input: &Value) -> Result<Value, String> {
    let Value::Number(text) = input else {
        return Err("gmtime() requires numeric inputs".to_string());
    };
    let ts = as_f64(text);
    let tm = broken_down(ts)?;
    let year = tm.tm_year.wrapping_add(1900);
    Ok(Value::Array(vec![
        field(year),
        field(tm.tm_mon),
        field(tm.tm_mday),
        field(tm.tm_hour),
        field(tm.tm_min),
        seconds(tm.tm_sec, ts),
        field(tm.tm_wday),
        field(tm.tm_yday),
    ]))
}

pub(crate) fn mktime(input: &Value) -> Result<Value, String> {
    let Value::Array(items) = input else {
        return Err("mktime requires array inputs".to_string());
    };
    // Six or fewer fields fail before the per-field check. Both failures
    // use the same message. Fields past the eighth are ignored.
    if items.len() <= 5 {
        return Err("mktime requires parsed datetime inputs".to_string());
    }
    let mut tm = match fields(items) {
        Some(tm) => tm,
        None => return Err("mktime requires parsed datetime inputs".to_string()),
    };
    stamp(&mut tm)
}

pub(crate) fn format_time(input: &Value, format: &Value) -> Result<Value, String> {
    match input {
        Value::Number(text) => {
            // A timestamp that does not convert fails before the format
            // is examined. jq aborts when the format is not a string after
            // a successful conversion. The message is the one jq uses for
            // an array.
            let mut tm = broken_down(as_f64(text))?;
            clear_zone(&mut tm);
            let text = require_format(format)?;
            render(&tm, text)
        }
        Value::Array(items) => {
            let text = require_format(format)?;
            let tm = match fields(items) {
                Some(tm) => tm,
                None => return Err("strftime/1 requires parsed datetime inputs".to_string()),
            };
            render(&tm, text)
        }
        _ => Err("strftime/1 requires parsed datetime inputs".to_string()),
    }
}

fn require_format(format: &Value) -> Result<&str, String> {
    match format {
        Value::String(text) => Ok(text),
        _ => Err("strftime/1 requires a string format".to_string()),
    }
}

/// Eight numbers. The year is stored as years since 1900. Each field is
/// truncated toward zero. `wday` and `yday` are required and then ignored
/// by `timegm`.
fn fields(items: &[Value]) -> Option<Tm> {
    if items.len() < 8 {
        return None;
    }
    let mut tm = blank();
    for index in 0..8 {
        let Value::Number(text) = &items[index] else {
            return None;
        };
        let raw = c_cast_i32(as_f64(text));
        match index {
            0 => tm.tm_year = raw.wrapping_sub(1900),
            1 => tm.tm_mon = raw,
            2 => tm.tm_mday = raw,
            3 => tm.tm_hour = raw,
            4 => tm.tm_min = raw,
            5 => tm.tm_sec = raw,
            6 => tm.tm_wday = raw,
            _ => tm.tm_yday = raw,
        }
    }
    Some(tm)
}

fn broken_down(ts: f64) -> Result<Tm, String> {
    let clock = c_cast_i64(ts);
    let mut tm = blank();
    let ptr = unsafe { gmtime_r(&clock, &mut tm) };
    if ptr.is_null() {
        Err("error converting number of seconds since epoch to datetime".to_string())
    } else {
        Ok(tm)
    }
}

fn stamp(tm: &mut Tm) -> Result<Value, String> {
    let clock = unsafe { timegm(tm) };
    // `-1` and `-2` are real timestamps as well as `timegm`'s sentinels.
    // jq reports the sentinel either way.
    if clock == -2 {
        Err("mktime not supported on this platform".to_string())
    } else if clock == -1 {
        Err("invalid gmtime representation".to_string())
    } else {
        Ok(computed(clock as f64))
    }
}

fn render(tm: &Tm, format: &str) -> Result<Value, String> {
    let visible = format.split('\0').next().unwrap_or("");
    check_letters(visible)?;
    let mut spec = Vec::with_capacity(visible.len() + 1);
    spec.extend_from_slice(visible.as_bytes());
    spec.push(0);
    // jq's buffer is `strlen + 100`. A result that does not fit, including
    // an empty format, is an unknown system failure.
    let cap = visible.len() + 0x64;
    let mut buf = vec![0u8; cap];
    let wrote = unsafe {
        strftime(
            buf.as_mut_ptr() as *mut c_char,
            cap,
            spec.as_ptr() as *const c_char,
            tm,
        )
    };
    if wrote == 0 || wrote >= cap {
        return Err("strftime/1: unknown system failure".to_string());
    }
    let text = String::from_utf8_lossy(&buf[..wrote]).into_owned();
    Ok(Value::String(text))
}

fn check_letters(format: &str) -> Result<(), String> {
    let mut chars = format.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            continue;
        }
        match chars.next() {
            Some('Y' | 'm' | 'd' | 'H' | 'M' | 'S' | 'Z') => {}
            Some(other) => return Err(format!("strftime does not support %{other}")),
            None => return Err("strftime does not support %".to_string()),
        }
    }
    Ok(())
}

fn trailing_space(end: *const c_char) -> bool {
    let byte = unsafe { *end as u8 };
    byte == 0 || unsafe { isspace(i32::from(byte)) } != 0
}

fn clear_zone(tm: &mut Tm) {
    tm.tm_isdst = 0;
    tm.tm_gmtoff = 0;
    tm.tm_zone = std::ptr::null();
}

fn blank() -> Tm {
    Tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 0,
        tm_mon: 0,
        tm_year: 0,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null(),
    }
}

/// The seconds field keeps the fraction `gmtime_r` dropped. The fraction
/// is `ts - floor(ts)`, so it stays in `[0, 1)` for a finite timestamp.
/// NaN stays NaN, and the calendar fields are the epoch.
fn seconds(tm_sec: i32, ts: f64) -> Value {
    let frac = ts - ts.floor();
    computed(f64::from(tm_sec) + frac)
}

fn field(n: i32) -> Value {
    computed(f64::from(n))
}

fn computed(n: f64) -> Value {
    Value::Number(number::mark_f64(number::format_jq_f64(n)))
}

fn as_f64(text: &str) -> f64 {
    match number::real(text) {
        number::Real::Nan => f64::NAN,
        number::Real::Num(n) => n,
    }
}

/// x86 `cvttsd2si` yields the indefinite integer for NaN and for a value
/// that does not fit. The other architectures saturate, and NaN becomes 0.
fn c_cast_i64(n: f64) -> i64 {
    if cfg!(any(target_arch = "x86", target_arch = "x86_64")) && out_of_i64(n) {
        i64::MIN
    } else {
        n as i64
    }
}

fn c_cast_i32(n: f64) -> i32 {
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
