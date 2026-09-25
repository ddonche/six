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

// --- file domain (Addendum A) -----------------------------------------------
//
// The v1 file domain is direct and one-shot: `in` observes filesystem state,
// `out` establishes state/location/absence. No open/close, no association.

use std::fs;
use std::path::Path;

/// One byte value (0..=255) as a Six Number.
fn byte(n: u8) -> Value {
    Value::Number(n as f64)
}

/// Parse a `["file" ...]` descriptor into (path, kind).
enum FileKind {
    Bare,      // ["file" path]                — removal target
    Text,      // ["file" path "text"]
    Binary,    // ["file" path "binary"]
    Directory, // ["file" path "directory"]
    Metadata,  // ["file" path "metadata"]
    Rename,    // ["file" source "path"]       — location change
}

fn parse_file(items: &[Value], line: usize) -> Result<(String, FileKind)> {
    let path = match items.get(1) {
        Some(Value::Text(p)) => p.clone(),
        _ => return Err(SixError::at(line, "file descriptor needs a text path: [\"file\" path ...]")),
    };
    let kind = match items.get(2) {
        None if items.len() == 2 => FileKind::Bare,
        Some(Value::Text(k)) if items.len() == 3 => match k.as_str() {
            "text" => FileKind::Text,
            "binary" => FileKind::Binary,
            "directory" => FileKind::Directory,
            "metadata" => FileKind::Metadata,
            "path" => FileKind::Rename,
            other => {
                return Err(SixError::at(line, format!("unknown file descriptor kind \"{}\"", other)));
            }
        },
        _ => return Err(SixError::at(line, "malformed file descriptor")),
    };
    Ok((path, kind))
}

/// `in(["file" ...])` — observe filesystem state.
pub fn file_in(items: &[Value], line: usize) -> Result<Value> {
    let (path, kind) = parse_file(items, line)?;
    let p = Path::new(&path);
    match kind {
        FileKind::Bare => Err(SixError::at(line, "in([\"file\" path]) is not valid; use \"text\", \"binary\", \"directory\", or \"metadata\"")),
        FileKind::Rename => Err(SixError::at(line, "in([\"file\" source \"path\"]) is not valid; \"path\" is an out-only location change")),
        FileKind::Text => match fs::metadata(p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Empty),
            Err(e) => Err(SixError::at(line, format!("file: cannot read {}: {}", path, e))),
            Ok(md) => {
                if md.is_dir() {
                    return Err(SixError::at(line, format!("file: {} is a directory, not a text file", path)));
                }
                let bytes = fs::read(p).map_err(|e| SixError::at(line, format!("file: cannot read {}: {}", path, e)))?;
                match String::from_utf8(bytes) {
                    Ok(s) => Ok(Value::Text(s)),
                    Err(_) => Err(SixError::at(line, format!("file: {} is not valid UTF-8", path))),
                }
            }
        },
        FileKind::Binary => match fs::metadata(p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Empty),
            Err(e) => Err(SixError::at(line, format!("file: cannot read {}: {}", path, e))),
            Ok(md) => {
                if md.is_dir() {
                    return Err(SixError::at(line, format!("file: {} is a directory, not a binary file", path)));
                }
                let bytes = fs::read(p).map_err(|e| SixError::at(line, format!("file: cannot read {}: {}", path, e)))?;
                Ok(Value::new_group(bytes.into_iter().map(byte).collect()))
            }
        },
        FileKind::Directory => match fs::metadata(p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Empty),
            Err(e) => Err(SixError::at(line, format!("file: cannot read {}: {}", path, e))),
            Ok(md) => {
                if !md.is_dir() {
                    return Err(SixError::at(line, format!("file: {} is not a directory", path)));
                }
                let mut names = Vec::new();
                for entry in fs::read_dir(p).map_err(|e| SixError::at(line, format!("file: cannot read directory {}: {}", path, e)))? {
                    let entry = entry.map_err(|e| SixError::at(line, format!("file: cannot read directory {}: {}", path, e)))?;
                    match entry.file_name().into_string() {
                        Ok(name) => names.push(Value::Text(name)),
                        Err(_) => return Err(SixError::at(line, format!("file: {} contains a name that is not valid text", path))),
                    }
                }
                Ok(Value::new_group(names))
            }
        },
        FileKind::Metadata => match fs::metadata(p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Empty),
            Err(e) => Err(SixError::at(line, format!("file: cannot read metadata for {}: {}", path, e))),
            Ok(md) => Ok(metadata_group(&md)),
        },
    }
}

/// `out(["file" ...] value)` — establish filesystem state, location, or absence.
pub fn file_out(items: &[Value], value: &Value, line: usize) -> Result<Value> {
    let (path, kind) = parse_file(items, line)?;
    let p = Path::new(&path);
    match kind {
        FileKind::Bare => {
            // Only `..` (establish absence / removal) is valid here.
            if !value.is_empty() {
                return Err(SixError::at(line, "out([\"file\" path] ..) removes an entry; only .. is accepted"));
            }
            match fs::symlink_metadata(p) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Empty), // already absent
                Err(e) => Err(SixError::at(line, format!("file: cannot remove {}: {}", path, e))),
                Ok(md) => {
                    let r = if md.is_dir() { fs::remove_dir(p) } else { fs::remove_file(p) };
                    r.map_err(|e| SixError::at(line, format!("file: cannot remove {}: {}", path, e)))?;
                    Ok(Value::Empty)
                }
            }
        }
        FileKind::Text => {
            let s = match value {
                Value::Text(s) => s,
                other => return Err(SixError::at(line, format!("file text write needs text, not a {}", other.type_name()))),
            };
            reject_dir(p, &path, line)?;
            fs::write(p, s.as_bytes()).map_err(|e| SixError::at(line, format!("file: cannot write {}: {}", path, e)))?;
            Ok(Value::Empty)
        }
        FileKind::Binary => {
            let bytes = value_to_bytes(value, line)?;
            reject_dir(p, &path, line)?;
            fs::write(p, &bytes).map_err(|e| SixError::at(line, format!("file: cannot write {}: {}", path, e)))?;
            Ok(Value::Empty)
        }
        FileKind::Directory => {
            let empty = matches!(value, Value::Group(g) if g.items.borrow().is_empty());
            if !empty {
                return Err(SixError::at(line, "creating a directory accepts only []"));
            }
            if fs::symlink_metadata(p).is_ok() {
                return Err(SixError::at(line, format!("file: {} already exists", path)));
            }
            // create_dir (not _all): parent must already exist.
            fs::create_dir(p).map_err(|e| SixError::at(line, format!("file: cannot create directory {}: {}", path, e)))?;
            Ok(Value::Empty)
        }
        FileKind::Rename => {
            let target = match value {
                Value::Text(t) => t.clone(),
                other => return Err(SixError::at(line, format!("file path change needs a text target, not a {}", other.type_name()))),
            };
            if fs::symlink_metadata(Path::new(&target)).is_ok() {
                return Err(SixError::at(line, format!("file: target {} already exists", target)));
            }
            fs::rename(p, Path::new(&target)).map_err(|e| SixError::at(line, format!("file: cannot move {} to {}: {}", path, target, e)))?;
            Ok(Value::Empty)
        }
        FileKind::Metadata => Err(SixError::at(line, "out to [\"file\" path \"metadata\"] is not supported in v1")),
    }
}

fn reject_dir(p: &Path, path: &str, line: usize) -> Result<()> {
    if let Ok(md) = fs::metadata(p) {
        if md.is_dir() {
            return Err(SixError::at(line, format!("file: {} is a directory", path)));
        }
    }
    Ok(())
}

fn value_to_bytes(value: &Value, line: usize) -> Result<Vec<u8>> {
    let g = match value {
        Value::Group(g) => g,
        other => return Err(SixError::at(line, format!("binary write needs a Group of bytes, not a {}", other.type_name()))),
    };
    let items = g.items.borrow();
    let mut bytes = Vec::with_capacity(items.len());
    for v in items.iter() {
        match v {
            Value::Number(n) if n.fract() == 0.0 && *n >= 0.0 && *n <= 255.0 => bytes.push(*n as u8),
            Value::Number(n) => return Err(SixError::at(line, format!("byte value must be an integer 0..255, got {}", n))),
            other => return Err(SixError::at(line, format!("byte value must be a number 0..255, not a {}", other.type_name()))),
        }
    }
    Ok(bytes)
}

fn metadata_group(md: &fs::Metadata) -> Value {
    let kind = if md.is_dir() { "directory" } else if md.is_file() { "file" } else { "other" };
    let size = if md.is_dir() { Value::Empty } else { Value::Number(md.len() as f64) };
    let modified = system_time_micros(md.modified().ok());
    let created = system_time_micros(md.created().ok());
    let readonly = Value::Bool(md.permissions().readonly());
    Value::new_group(vec![
        pair("kind", Value::Text(kind.to_string())),
        pair("size", size),
        pair("modified", modified),
        pair("created", created),
        pair("readonly", readonly),
    ])
}

fn pair(key: &str, value: Value) -> Value {
    Value::new_group(vec![Value::Text(key.to_string()), value])
}

fn system_time_micros(t: Option<SystemTime>) -> Value {
    match t.and_then(|t| t.duration_since(UNIX_EPOCH).ok()) {
        Some(d) => Value::Number(d.as_micros() as f64),
        None => Value::Empty,
    }
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
