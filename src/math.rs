//! jq's math functions. Each one calls the C library, so a Mac and a
//! Linux build follow the jq beside them, including `gamma`.

use crate::number;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unary {
    Acos,
    Acosh,
    Asin,
    Asinh,
    Atan,
    Atanh,
    Cbrt,
    Ceil,
    Cos,
    Cosh,
    Erf,
    Erfc,
    Exp,
    Exp10,
    Exp2,
    Expm1,
    Fabs,
    Floor,
    Frexp,
    Gamma,
    J0,
    J1,
    Lgamma,
    Log,
    Log10,
    Log1p,
    Log2,
    Logb,
    Modf,
    Nearbyint,
    Rint,
    Round,
    Significand,
    Sin,
    Sinh,
    Sqrt,
    Tan,
    Tanh,
    Tgamma,
    Trunc,
    Y0,
    Y1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binary {
    Atan2,
    Copysign,
    Drem,
    Fdim,
    Fmax,
    Fmin,
    Fmod,
    Hypot,
    Jn,
    Ldexp,
    Nextafter,
    Pow,
    Remainder,
    Scalb,
    Scalbln,
    Yn,
}

pub enum Kind {
    Unary(Unary),
    Binary(Binary),
    Fma,
}

pub enum Out {
    Num(f64),
    /// `frexp` is `[mantissa, exponent]`. `modf` is `[fraction, integer]`.
    Pair(f64, f64),
}

pub fn kind(name: &str) -> Option<Kind> {
    Some(match name {
        "acos" => Kind::Unary(Unary::Acos),
        "acosh" => Kind::Unary(Unary::Acosh),
        "asin" => Kind::Unary(Unary::Asin),
        "asinh" => Kind::Unary(Unary::Asinh),
        "atan" => Kind::Unary(Unary::Atan),
        "atanh" => Kind::Unary(Unary::Atanh),
        "cbrt" => Kind::Unary(Unary::Cbrt),
        "ceil" => Kind::Unary(Unary::Ceil),
        "cos" => Kind::Unary(Unary::Cos),
        "cosh" => Kind::Unary(Unary::Cosh),
        "erf" => Kind::Unary(Unary::Erf),
        "erfc" => Kind::Unary(Unary::Erfc),
        "exp" => Kind::Unary(Unary::Exp),
        "exp10" => Kind::Unary(Unary::Exp10),
        "exp2" => Kind::Unary(Unary::Exp2),
        "expm1" => Kind::Unary(Unary::Expm1),
        "fabs" => Kind::Unary(Unary::Fabs),
        "floor" => Kind::Unary(Unary::Floor),
        "frexp" => Kind::Unary(Unary::Frexp),
        "gamma" => Kind::Unary(Unary::Gamma),
        "j0" => Kind::Unary(Unary::J0),
        "j1" => Kind::Unary(Unary::J1),
        "lgamma" => Kind::Unary(Unary::Lgamma),
        "log" => Kind::Unary(Unary::Log),
        "log10" => Kind::Unary(Unary::Log10),
        "log1p" => Kind::Unary(Unary::Log1p),
        "log2" => Kind::Unary(Unary::Log2),
        "logb" => Kind::Unary(Unary::Logb),
        "modf" => Kind::Unary(Unary::Modf),
        "nearbyint" => Kind::Unary(Unary::Nearbyint),
        "rint" => Kind::Unary(Unary::Rint),
        "round" => Kind::Unary(Unary::Round),
        "significand" => Kind::Unary(Unary::Significand),
        "sin" => Kind::Unary(Unary::Sin),
        "sinh" => Kind::Unary(Unary::Sinh),
        "sqrt" => Kind::Unary(Unary::Sqrt),
        "tan" => Kind::Unary(Unary::Tan),
        "tanh" => Kind::Unary(Unary::Tanh),
        "tgamma" => Kind::Unary(Unary::Tgamma),
        "trunc" => Kind::Unary(Unary::Trunc),
        "y0" => Kind::Unary(Unary::Y0),
        "y1" => Kind::Unary(Unary::Y1),
        "atan2" => Kind::Binary(Binary::Atan2),
        "copysign" => Kind::Binary(Binary::Copysign),
        "drem" => Kind::Binary(Binary::Drem),
        "fdim" => Kind::Binary(Binary::Fdim),
        "fmax" => Kind::Binary(Binary::Fmax),
        "fmin" => Kind::Binary(Binary::Fmin),
        "fmod" => Kind::Binary(Binary::Fmod),
        "hypot" => Kind::Binary(Binary::Hypot),
        "jn" => Kind::Binary(Binary::Jn),
        "ldexp" => Kind::Binary(Binary::Ldexp),
        "nextafter" => Kind::Binary(Binary::Nextafter),
        "pow" => Kind::Binary(Binary::Pow),
        "remainder" => Kind::Binary(Binary::Remainder),
        "scalb" => Kind::Binary(Binary::Scalb),
        "scalbln" => Kind::Binary(Binary::Scalbln),
        "yn" => Kind::Binary(Binary::Yn),
        "fma" => Kind::Fma,
        _ => return None,
    })
}

pub fn unary(op: Unary, n: f64) -> Out {
    if matches!(op, Unary::Frexp) {
        let mut exp = 0;
        let frac = unsafe { frexp(n, &mut exp) };
        return Out::Pair(frac, f64::from(exp));
    }
    if matches!(op, Unary::Modf) {
        let mut whole = 0.0;
        let frac = unsafe { modf(n, &mut whole) };
        return Out::Pair(frac, whole);
    }
    let value = unsafe {
        match op {
            Unary::Acos => acos(n),
            Unary::Acosh => acosh(n),
            Unary::Asin => asin(n),
            Unary::Asinh => asinh(n),
            Unary::Atan => atan(n),
            Unary::Atanh => atanh(n),
            Unary::Cbrt => cbrt(n),
            Unary::Ceil => ceil(n),
            Unary::Cos => cos(n),
            Unary::Cosh => cosh(n),
            Unary::Erf => erf(n),
            Unary::Erfc => erfc(n),
            Unary::Exp => exp(n),
            Unary::Exp10 => exp10_value(n),
            Unary::Exp2 => exp2(n),
            Unary::Expm1 => expm1(n),
            Unary::Fabs => fabs(n),
            Unary::Floor => floor(n),
            Unary::Gamma => gamma_value(n),
            Unary::J0 => j0(n),
            Unary::J1 => j1(n),
            Unary::Lgamma => lgamma(n),
            Unary::Log => log(n),
            Unary::Log10 => log10(n),
            Unary::Log1p => log1p(n),
            Unary::Log2 => log2(n),
            Unary::Logb => logb(n),
            Unary::Nearbyint => nearbyint(n),
            Unary::Rint => rint(n),
            Unary::Round => round(n),
            Unary::Significand => significand_value(n),
            Unary::Sin => sin(n),
            Unary::Sinh => sinh(n),
            Unary::Sqrt => sqrt(n),
            Unary::Tan => tan(n),
            Unary::Tanh => tanh(n),
            Unary::Tgamma => tgamma(n),
            Unary::Trunc => trunc(n),
            Unary::Y0 => y0(n),
            Unary::Y1 => y1(n),
            Unary::Frexp | Unary::Modf => unreachable!("pair results return above"),
        }
    };
    Out::Num(value)
}

pub fn binary(op: Binary, a: f64, b: f64) -> f64 {
    unsafe {
        match op {
            Binary::Atan2 => atan2(a, b),
            Binary::Copysign => copysign(a, b),
            Binary::Drem => drem_value(a, b),
            Binary::Fdim => fdim(a, b),
            Binary::Fmax => fmax(a, b),
            Binary::Fmin => fmin(a, b),
            Binary::Fmod => fmod(a, b),
            Binary::Hypot => hypot(a, b),
            Binary::Jn => jn(number::c_cast_i32(a), b),
            Binary::Ldexp => ldexp(a, number::c_cast_i32(b)),
            Binary::Nextafter => nextafter(a, b),
            Binary::Pow => pow(a, b),
            Binary::Remainder => remainder(a, b),
            Binary::Scalb => scalb(a, b),
            Binary::Scalbln => scalbln(a, number::c_cast_i64(b)),
            Binary::Yn => yn(number::c_cast_i32(a), b),
        }
    }
}

pub fn fma(x: f64, y: f64, z: f64) -> f64 {
    unsafe { fma_c(x, y, z) }
}

fn exp10_value(x: f64) -> f64 {
    unsafe {
        // macOS exports this as `__exp10`. glibc exports `exp10`.
        #[cfg(target_os = "macos")]
        {
            exp10_mac(x)
        }
        #[cfg(not(target_os = "macos"))]
        {
            exp10(x)
        }
    }
}

fn gamma_value(x: f64) -> f64 {
    unsafe {
        // The macOS header replaces `gamma` with `tgamma`, and that is what
        // this platform's jq returns. glibc's `gamma` is a different function.
        #[cfg(target_os = "macos")]
        {
            tgamma(x)
        }
        #[cfg(not(target_os = "macos"))]
        {
            gamma(x)
        }
    }
}

fn significand_value(x: f64) -> f64 {
    // arm64 libm has no `significand`. `2 * frexp` is the value jq returns,
    // including zero, negative zero, and a subnormal.
    #[cfg(target_os = "macos")]
    {
        let mut exp = 0;
        let frac = unsafe { frexp(x, &mut exp) };
        return frac * 2.0;
    }
    #[cfg(not(target_os = "macos"))]
    unsafe {
        significand(x)
    }
}

fn drem_value(x: f64, y: f64) -> f64 {
    unsafe {
        // The macOS header replaces `drem` with `remainder`.
        #[cfg(target_os = "macos")]
        {
            remainder(x, y)
        }
        #[cfg(not(target_os = "macos"))]
        {
            drem(x, y)
        }
    }
}

#[link(name = "m")]
extern "C" {
    fn acos(x: f64) -> f64;
    fn acosh(x: f64) -> f64;
    fn asin(x: f64) -> f64;
    fn asinh(x: f64) -> f64;
    fn atan(x: f64) -> f64;
    fn atanh(x: f64) -> f64;
    fn atan2(y: f64, x: f64) -> f64;
    fn cbrt(x: f64) -> f64;
    fn ceil(x: f64) -> f64;
    fn copysign(x: f64, y: f64) -> f64;
    fn cos(x: f64) -> f64;
    fn cosh(x: f64) -> f64;
    fn erf(x: f64) -> f64;
    fn erfc(x: f64) -> f64;
    fn exp(x: f64) -> f64;
    #[cfg(target_os = "macos")]
    #[link_name = "__exp10"]
    fn exp10_mac(x: f64) -> f64;
    #[cfg(not(target_os = "macos"))]
    fn exp10(x: f64) -> f64;
    fn exp2(x: f64) -> f64;
    fn expm1(x: f64) -> f64;
    fn fabs(x: f64) -> f64;
    fn fdim(x: f64, y: f64) -> f64;
    fn floor(x: f64) -> f64;
    #[link_name = "fma"]
    fn fma_c(x: f64, y: f64, z: f64) -> f64;
    fn fmax(x: f64, y: f64) -> f64;
    fn fmin(x: f64, y: f64) -> f64;
    fn fmod(x: f64, y: f64) -> f64;
    fn frexp(x: f64, exp: *mut i32) -> f64;
    #[cfg(not(target_os = "macos"))]
    fn gamma(x: f64) -> f64;
    fn hypot(x: f64, y: f64) -> f64;
    fn j0(x: f64) -> f64;
    fn j1(x: f64) -> f64;
    fn jn(n: i32, x: f64) -> f64;
    fn ldexp(x: f64, exp: i32) -> f64;
    fn lgamma(x: f64) -> f64;
    fn log(x: f64) -> f64;
    fn log10(x: f64) -> f64;
    fn log1p(x: f64) -> f64;
    fn log2(x: f64) -> f64;
    fn logb(x: f64) -> f64;
    fn modf(x: f64, iptr: *mut f64) -> f64;
    fn nearbyint(x: f64) -> f64;
    fn nextafter(x: f64, y: f64) -> f64;
    fn pow(x: f64, y: f64) -> f64;
    fn remainder(x: f64, y: f64) -> f64;
    fn rint(x: f64) -> f64;
    fn round(x: f64) -> f64;
    fn scalb(x: f64, n: f64) -> f64;
    fn scalbln(x: f64, n: i64) -> f64;
    #[cfg(not(target_os = "macos"))]
    fn significand(x: f64) -> f64;
    #[cfg(not(target_os = "macos"))]
    fn drem(x: f64, y: f64) -> f64;
    fn sin(x: f64) -> f64;
    fn sinh(x: f64) -> f64;
    fn sqrt(x: f64) -> f64;
    fn tan(x: f64) -> f64;
    fn tanh(x: f64) -> f64;
    fn tgamma(x: f64) -> f64;
    fn trunc(x: f64) -> f64;
    fn y0(x: f64) -> f64;
    fn y1(x: f64) -> f64;
    fn yn(n: i32, x: f64) -> f64;
}
