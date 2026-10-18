//! `todate`, `fromdate`, `gmtime`, `localtime`, `mktime`, `strftime`,
//! `strflocaltime`, `strptime`, and `now`.
//!
//! Broken-down times go through the C library, so each platform agrees
//! with the jq it ships. Apple's jq then replaces the weekday and yearday
//! `strptime` stored. `strftime`, `strflocaltime`, and `strptime` accept
//! `%Y` `%m` `%d` `%H` `%M` `%S` `%Z` `%j` `%a` `%A` `%b` `%B` `%e` `%y`
//! `%z` `%I` `%p` `%u` `%w` `%s` `%D` `%F` `%T` `%R` `%c` `%C` `%g` `%G`
//! `%h` `%k` `%l` `%n` `%r` `%t` `%U` `%V` `%W` `%x` `%X` `%P` and `%%`.

use crate::number;
use crate::value::Value;
use std::ffi::{c_char, c_void};

const ISO: &str = "%Y-%m-%dT%H:%M:%SZ";

/// Days before each month, not counting a leap day.
#[cfg(target_os = "macos")]
const BEFORE_MONTH: [i32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

/// `struct tm` on macOS and glibc, LP64. `gmtime_r` and `localtime_r`
/// fill `tm_zone`. jq clears that pointer and the offset before
/// formatting, so `%Z` is the local standard abbreviation. `%z` is what
/// the C library prints for a cleared offset. `strftime` keeps the UTC
/// calendar. `strflocaltime` keeps the local calendar.
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

/// `struct timeval`. `tv_usec` is 32 bits on macOS and 64 bits on glibc.
#[cfg(target_os = "macos")]
#[repr(C)]
struct Timeval {
    tv_sec: i64,
    tv_usec: i32,
}

#[cfg(target_os = "linux")]
#[repr(C)]
struct Timeval {
    tv_sec: i64,
    tv_usec: i64,
}

const _: () = {
    assert!(std::mem::size_of::<Timeval>() == 16);
    assert!(std::mem::align_of::<Timeval>() == 8);
};

extern "C" {
    fn gmtime_r(clock: *const i64, out: *mut Tm) -> *mut Tm;
    fn localtime_r(clock: *const i64, out: *mut Tm) -> *mut Tm;
    fn timegm(tm: *mut Tm) -> i64;
    /// The C library keeps these for the whole process. `localtime` replaces
    /// the numeric offset a fresh process would print.
    static mut tzname: [*mut c_char; 2];
    fn strftime(buf: *mut c_char, max: usize, format: *const c_char, tm: *const Tm) -> usize;
    fn strptime(text: *const c_char, format: *const c_char, tm: *mut Tm) -> *mut c_char;
    fn setlocale(category: i32, locale: *const c_char) -> *mut c_char;
    fn isspace(c: i32) -> i32;
    fn gettimeofday(tv: *mut Timeval, tz: *mut c_void) -> i32;
    fn time(tloc: *mut i64) -> i64;
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
        return Err(mismatch(text, ISO));
    }
    // `timegm` ignores the weekday and yearday `strptime` stored.
    stamp(&mut tm)
}

pub(crate) fn gmtime(input: &Value) -> Result<Value, String> {
    clock_array(input, false, "gmtime() requires numeric inputs")
}

pub(crate) fn localtime(input: &Value) -> Result<Value, String> {
    clock_array(input, true, "localtime() requires numeric inputs")
}

fn clock_array(input: &Value, local: bool, bad_type: &str) -> Result<Value, String> {
    let Value::Number(text) = input else {
        return Err(bad_type.to_string());
    };
    let ts = as_f64(text);
    let tm = broken_down(ts, local)?;
    let year = tm.tm_year.wrapping_add(1900);
    Ok(Value::array(vec![
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
    format_clock(input, format, "strftime", false)
}

pub(crate) fn strflocaltime(input: &Value, format: &Value) -> Result<Value, String> {
    format_clock(input, format, "strflocaltime", true)
}

fn format_clock(input: &Value, format: &Value, name: &str, local: bool) -> Result<Value, String> {
    match input {
        Value::Number(text) => {
            // A timestamp that does not convert fails before the format
            // is examined. jq aborts when the format is not a string after
            // a successful conversion. The message is the one jq uses for
            // an array.
            let mut tm = broken_down(as_f64(text), local)?;
            // The calendar stays as `gmtime_r` or `localtime_r` filled it.
            // jq clears the zone, so a summer stamp does not keep that
            // date's daylight-saving abbreviation. `%z` is then whatever
            // the C library prints for a cleared offset.
            clear_zone(&mut tm);
            let text = require_format(format, name)?;
            render(&tm, text, name)
        }
        Value::Array(items) => {
            let text = require_format(format, name)?;
            let tm = match fields(items) {
                Some(tm) => tm,
                None => return Err(parsed(name)),
            };
            render(&tm, text, name)
        }
        _ => Err(parsed(name)),
    }
}

fn parsed(name: &str) -> String {
    format!("{name}/1 requires parsed datetime inputs")
}

pub(crate) fn strptime_fmt(input: &Value, format: &Value) -> Result<Value, String> {
    let (text, fmt) = match (input, format) {
        (Value::String(text), Value::String(fmt)) => (text.as_str(), fmt.as_str()),
        _ => return Err("strptime/1 requires string inputs and arguments".to_string()),
    };
    let visible = fmt.split('\0').next().unwrap_or("");
    check_letters(visible, "strptime")?;
    let mut raw = text.as_bytes().to_vec();
    raw.push(0);
    let mut spec = Vec::with_capacity(visible.len() + 1);
    spec.extend_from_slice(visible.as_bytes());
    spec.push(0);
    // 8 and 367 are jq's unset weekday and yearday. `strptime` leaves
    // whichever fields the format does not write.
    let mut tm = blank();
    tm.tm_wday = 8;
    tm.tm_yday = 367;
    let end = with_tz(|| unsafe {
        strptime(
            raw.as_ptr() as *const c_char,
            spec.as_ptr() as *const c_char,
            &mut tm,
        )
    });
    if end.is_null() || !trailing_space(end) {
        return Err(mismatch(text, visible));
    }
    // Apple's jq replaces the weekday and yearday with its own calculation.
    // Other builds keep the fields `strptime` stored, including the 8 and
    // 367 sentinels when the format wrote neither.
    #[cfg(target_os = "macos")]
    set_calendar(&mut tm);
    let mut items = civil_fields(&tm);
    if unsafe { *end } != 0 {
        items.push(Value::String(c_str(end)));
    }
    Ok(Value::array(items))
}

pub(crate) fn now() -> Value {
    computed(clock_now())
}

fn require_format<'a>(format: &'a Value, name: &str) -> Result<&'a str, String> {
    match format {
        Value::String(text) => Ok(text),
        _ => Err(format!("{name}/1 requires a string format")),
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
        let raw = number::c_cast_i32(as_f64(text));
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

fn broken_down(ts: f64, local: bool) -> Result<Tm, String> {
    let clock = number::c_cast_i64(ts);
    let mut tm = blank();
    let ptr = with_tz(|| unsafe {
        if local {
            localtime_r(&clock, &mut tm)
        } else {
            gmtime_r(&clock, &mut tm)
        }
    });
    if ptr.is_null() {
        Err("error converting number of seconds since epoch to datetime".to_string())
    } else {
        Ok(tm)
    }
}

fn stamp(tm: &mut Tm) -> Result<Value, String> {
    let clock = with_tz(|| unsafe { timegm(tm) });
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

fn render(tm: &Tm, format: &str, name: &str) -> Result<Value, String> {
    let visible = format.split('\0').next().unwrap_or("");
    check_letters(visible, name)?;
    let mut spec = Vec::with_capacity(visible.len() + 1);
    spec.extend_from_slice(visible.as_bytes());
    spec.push(0);
    // jq's buffer is `strlen + 100`. A result that does not fit, including
    // an empty format, is an unknown system failure.
    let cap = visible.len() + 0x64;
    let mut buf = vec![0u8; cap];
    let wrote = with_tz(|| unsafe {
        strftime(
            buf.as_mut_ptr() as *mut c_char,
            cap,
            spec.as_ptr() as *const c_char,
            tm,
        )
    });
    if wrote == 0 || wrote >= cap {
        return Err(format!("{name}/1: unknown system failure"));
    }
    let text = String::from_utf8_lossy(&buf[..wrote]).into_owned();
    Ok(Value::String(text))
}

fn check_letters(format: &str, filter: &str) -> Result<(), String> {
    let mut chars = format.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            continue;
        }
        match chars.next() {
            Some(
                'Y' | 'm' | 'd' | 'H' | 'M' | 'S' | 'Z' | 'j' | 'a' | 'A' | 'b' | 'B' | 'e' | 'y'
                | 'z' | 'I' | 'p' | 'u' | 'w' | 's' | 'D' | 'F' | 'T' | 'R' | 'c' | 'C' | 'g' | 'G'
                | 'h' | 'k' | 'l' | 'n' | 'r' | 't' | 'U' | 'V' | 'W' | 'x' | 'X' | 'P' | '%',
            ) => {}
            Some(other) => return Err(format!("{filter} does not support %{other}")),
            None => return Err(format!("{filter} does not support %")),
        }
    }
    Ok(())
}

fn mismatch(text: &str, format: &str) -> String {
    // jq prints both with `%s`, which stops at the first NUL.
    let shown = text.split('\0').next().unwrap_or("");
    let spec = format.split('\0').next().unwrap_or("");
    format!("date \"{shown}\" does not match format \"{spec}\"")
}

#[cfg(target_os = "macos")]
fn set_calendar(tm: &mut Tm) {
    let year = tm.tm_year.wrapping_add(1900);
    tm.tm_wday = weekday(year, tm.tm_mon, tm.tm_mday);
    tm.tm_yday = yearday(tm.tm_year, tm.tm_mon, tm.tm_mday);
}

/// Sunday is 0. March is month 1 in the month term, and January and
/// February belong to the previous year. The month term is
/// `floor(2.6 * m - 0.2)`.
#[cfg(target_os = "macos")]
fn weekday(year: i32, mon: i32, mday: i32) -> i32 {
    let century = year / 100;
    let yy = year % 100;
    let early = mon < 2;
    let month_index = if early {
        mon.wrapping_add(11)
    } else {
        mon.wrapping_sub(1)
    };
    let yy_adj = if early { yy.wrapping_sub(1) } else { yy };
    let total = mday
        .wrapping_sub(century.wrapping_mul(2))
        .wrapping_add(yy_adj)
        .wrapping_add(yy_adj.div_euclid(4))
        .wrapping_add(century.div_euclid(4))
        .wrapping_add(month_term(month_index));
    let rem = total % 7;
    if rem < 0 {
        rem + 7
    } else {
        rem
    }
}

#[cfg(target_os = "macos")]
fn month_term(month_index: i32) -> i32 {
    (f64::from(month_index) * 2.6 - 0.2).floor() as i32
}

/// Days since January 1. A day of 0 is one less than the first of the
/// month. Months before March do not add the leap day.
#[cfg(target_os = "macos")]
fn yearday(tm_year: i32, mon: i32, mday: i32) -> i32 {
    let adjust: i32 = if mon < 2 || !leap(tm_year.wrapping_add(1900)) {
        -1
    } else {
        0
    };
    let abs_m = if mon < 0 { mon.wrapping_neg() } else { mon };
    let index = if (0..12).contains(&abs_m) {
        abs_m as usize
    } else {
        (abs_m as u32 % 12) as usize
    };
    adjust.wrapping_add(mday).wrapping_add(BEFORE_MONTH[index])
}

#[cfg(target_os = "macos")]
fn leap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn civil_fields(tm: &Tm) -> Vec<Value> {
    let year = tm.tm_year.wrapping_add(1900);
    vec![
        field(year),
        field(tm.tm_mon),
        field(tm.tm_mday),
        field(tm.tm_hour),
        field(tm.tm_min),
        field(tm.tm_sec),
        field(tm.tm_wday),
        field(tm.tm_yday),
    ]
}

fn c_str(start: *const c_char) -> String {
    let mut len = 0usize;
    unsafe {
        while *start.add(len) != 0 {
            len += 1;
        }
        let bytes = std::slice::from_raw_parts(start as *const u8, len);
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn clock_now() -> f64 {
    let mut tv = Timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    let rc = unsafe { gettimeofday(&mut tv, std::ptr::null_mut()) };
    if rc == -1 {
        return unsafe { time(std::ptr::null_mut()) } as f64;
    }
    tv.tv_sec as f64 + (tv.tv_usec as f64) / 1_000_000.0
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

/// The seconds field keeps the fraction the C library dropped. The fraction
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
    number::f64_of(text)
}

/// One run's timezone names. A fresh process prints the numeric offset.
/// `localtime` replaces that, for this run only, with the abbreviation.
struct RunZone {
    buf0: [u8; 64],
    buf1: [u8; 64],
}

thread_local! {
    static RUN_ZONE: std::cell::RefCell<Option<RunZone>> = const { std::cell::RefCell::new(None) };
}

/// The names a fresh process prints, captured once.
static FRESH_ZONE: std::sync::OnceLock<(Vec<u8>, Vec<u8>)> = std::sync::OnceLock::new();

/// Serializes the process-global `tzname` while a run borrows it.
static TZ_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) struct ZoneGuard;

/// Remember the fresh zone names for this run. Later runs do not see a
/// `localtime` that happened in an earlier one.
pub(crate) fn zone_guard() -> ZoneGuard {
    // Set the locale before the first `strftime`, which fills `tzname`.
    let (saved0, saved1) = {
        let _hold = TZ_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        use_process_locale();
        fresh_names()
    };
    RUN_ZONE.with(|slot| {
        let mut zone = RunZone {
            buf0: [0; 64],
            buf1: [0; 64],
        };
        copy_into(&mut zone.buf0, &saved0);
        copy_into(&mut zone.buf1, &saved1);
        *slot.borrow_mut() = Some(zone);
    });
    ZoneGuard
}

impl Drop for ZoneGuard {
    fn drop(&mut self) {
        let _hold = TZ_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        RUN_ZONE.with(|slot| *slot.borrow_mut() = None);
        point_at_fresh();
    }
}

fn fresh_names() -> (Vec<u8>, Vec<u8>) {
    FRESH_ZONE
        .get_or_init(|| {
            prime_tzname();
            unsafe { (copy_cstr(tzname[0]), copy_cstr(tzname[1])) }
        })
        .clone()
}

/// `LC_ALL` in `locale.h`. glibc numbers the categories differently.
#[cfg(target_os = "macos")]
const LC_ALL: i32 = 0;
#[cfg(target_os = "linux")]
const LC_ALL: i32 = 6;

/// An empty locale name is the C library's request for the locale jq uses.
/// `LC_ALL` wins when it names a locale, then `LC_TIME`, then `LANG`. An
/// unknown name leaves the C locale in place.
fn use_process_locale() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let empty = [0u8];
        unsafe {
            setlocale(LC_ALL, empty.as_ptr() as *const c_char);
        }
    });
}

/// The first `strftime` of a fresh process fills `tzname`.
fn prime_tzname() {
    let clock = 0i64;
    let mut tm = blank();
    unsafe {
        gmtime_r(&clock, &mut tm);
    }
    clear_zone(&mut tm);
    let spec = b"%Z\0";
    let mut buf = [0u8; 32];
    unsafe {
        strftime(
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
            spec.as_ptr() as *const c_char,
            &tm,
        );
    }
}

/// Run `body` with this run's zone names. The real binary has no saved names
/// and uses the process as it is.
fn with_tz<T>(body: impl FnOnce() -> T) -> T {
    let active = RUN_ZONE.with(|slot| slot.borrow().is_some());
    if !active {
        use_process_locale();
        return body();
    }
    let _hold = TZ_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    use_process_locale();
    point_at_run();
    let out = body();
    save_run();
    out
}

fn point_at_run() {
    RUN_ZONE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let zone = slot.as_mut().expect("zone");
        unsafe {
            tzname[0] = zone.buf0.as_mut_ptr() as *mut c_char;
            tzname[1] = zone.buf1.as_mut_ptr() as *mut c_char;
        }
    });
}

fn point_at_fresh() {
    let Some((saved0, saved1)) = FRESH_ZONE.get() else {
        return;
    };
    // The buffers have to outlive this function. Leak one stable copy.
    static LIVE: std::sync::OnceLock<(Vec<u8>, Vec<u8>)> = std::sync::OnceLock::new();
    let (live0, live1) = LIVE.get_or_init(|| (saved0.clone(), saved1.clone()));
    unsafe {
        tzname[0] = live0.as_ptr() as *mut c_char;
        tzname[1] = live1.as_ptr() as *mut c_char;
    }
}

fn save_run() {
    let name0 = unsafe { copy_cstr(tzname[0]) };
    let name1 = unsafe { copy_cstr(tzname[1]) };
    RUN_ZONE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(zone) = slot.as_mut() else {
            return;
        };
        copy_into(&mut zone.buf0, &name0);
        copy_into(&mut zone.buf1, &name1);
    });
}

fn copy_into(dst: &mut [u8], src: &[u8]) {
    dst.fill(0);
    let n = src.len().min(dst.len());
    dst[..n].copy_from_slice(&src[..n]);
    if n == dst.len() {
        dst[dst.len() - 1] = 0;
    }
}

fn copy_cstr(ptr: *const c_char) -> Vec<u8> {
    if ptr.is_null() {
        return vec![0];
    }
    let mut out = Vec::new();
    unsafe {
        for index in 0..63 {
            let byte = *ptr.add(index) as u8;
            out.push(byte);
            if byte == 0 {
                return out;
            }
        }
    }
    out.push(0);
    out
}
