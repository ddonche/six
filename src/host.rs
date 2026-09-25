//! The host boundary — Six's window to the outside world.
//!
//! This module will grow to hold the four relationship primitives
//! (`open`/`in`/`out`/`close`) and the runtime channels. For now it holds the
//! two pure observations, `entropy` and `time`, which depend on nothing else in
//! the host model.
//!
//! Neither introduces a Six value type: both return an ordinary `Number`.

use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::error::{Result, SixError};
use crate::value::Value;

/// A hidden runtime association bound to a Group's *identity* (never to its
/// visible contents). This is what turns an ordinary Group into a live host
/// relationship. It is stored `Box`ed behind an `Option` on `GroupData`, so a
/// plain Group carries only a null pointer's worth of overhead.
///
/// Aliasing shares the association (same `Rc<GroupData>`); `::` deep-copy makes
/// a fresh `GroupData` with no association; visible mutation never touches it.
#[derive(Debug)]
pub enum Assoc {
    /// The program's runtime input channel (`in` / `close`).
    RuntimeInput,
    /// The program's runtime output channel (`out` / `close`).
    RuntimeOutput,
    /// The program's runtime diagnostic channel (`out` / `close`).
    RuntimeError,
    /// A relationship that has been closed. Any further use is a runtime error;
    /// closing again is a runtime error. Kept distinct from "no association" so
    /// the error message can tell a closed relationship from a deep copy.
    Closed,
}

impl Assoc {
    /// A short human name for error messages.
    pub fn describe(&self) -> &'static str {
        match self {
            Assoc::RuntimeInput => "input channel",
            Assoc::RuntimeOutput => "output channel",
            Assoc::RuntimeError => "error channel",
            Assoc::Closed => "closed relationship",
        }
    }
}

/// 2^53 - 1 = 9,007,199,254,740,991 — every integer through here is exactly
/// representable as an f64.
const MAX_53: u64 = (1u64 << 53) - 1;

/// `entropy()` — one uniformly distributed integer in `0 ..= 2^53-1`, drawn from
/// a cryptographically secure OS source. There is no weak fallback: if secure
/// randomness cannot be obtained, this is a runtime error.
pub fn entropy(line: usize) -> Result<Value> {
    let bits = os_random_u64(line)? & MAX_53;
    Ok(Value::Number(bits as f64))
}

/// `time(mode)` — integer microseconds. `"utc"` since the Unix epoch (may move
/// backward), `"steady"` from a monotonic origin (never decreases).
pub fn time(mode: &str, line: usize) -> Result<Value> {
    match mode {
        "utc" => {
            let micros = match SystemTime::now().duration_since(UNIX_EPOCH) {
                Ok(d) => d.as_micros() as f64,
                // Before the epoch: UTC is allowed to be negative.
                Err(e) => -(e.duration().as_micros() as f64),
            };
            Ok(Value::Number(micros))
        }
        "steady" => {
            let micros = steady_base().elapsed().as_micros() as f64;
            Ok(Value::Number(micros))
        }
        other => Err(SixError::at(
            line,
            format!("time mode must be \"utc\" or \"steady\", not \"{}\"", other),
        )),
    }
}

/// A process-wide monotonic origin for `time("steady")`. Its absolute value is
/// meaningless; only differences matter.
fn steady_base() -> Instant {
    static BASE: OnceLock<Instant> = OnceLock::new();
    *BASE.get_or_init(Instant::now)
}

// --- secure OS randomness ---------------------------------------------------

#[cfg(unix)]
fn os_random_u64(line: usize) -> Result<u64> {
    use std::io::Read;
    let mut file = std::fs::File::open("/dev/urandom")
        .map_err(|e| SixError::at(line, format!("entropy: secure randomness unavailable: {}", e)))?;
    let mut buf = [0u8; 8];
    file.read_exact(&mut buf)
        .map_err(|e| SixError::at(line, format!("entropy: secure randomness read failed: {}", e)))?;
    Ok(u64::from_le_bytes(buf))
}

#[cfg(windows)]
fn os_random_u64(line: usize) -> Result<u64> {
    // BCryptGenRandom with the system-preferred RNG needs no algorithm handle.
    #[link(name = "bcrypt")]
    extern "system" {
        fn BCryptGenRandom(h_algorithm: *mut core::ffi::c_void, pb_buffer: *mut u8, cb_buffer: u32, dw_flags: u32) -> i32;
    }
    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;
    let mut buf = [0u8; 8];
    let status = unsafe {
        BCryptGenRandom(core::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG)
    };
    if status != 0 {
        return Err(SixError::at(
            line,
            format!("entropy: secure randomness unavailable (BCryptGenRandom status {})", status),
        ));
    }
    Ok(u64::from_le_bytes(buf))
}

#[cfg(not(any(unix, windows)))]
fn os_random_u64(line: usize) -> Result<u64> {
    Err(SixError::at(line, "entropy: no secure randomness source on this platform"))
}
