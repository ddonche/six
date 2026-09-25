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
    /// A TCP connection (`in` / `out` / `close`).
    Tcp(TcpConn),
    /// A TCP listener (`open` to accept / `close`).
    TcpListener(TcpListenerState),
    /// A bound UDP socket (`in` / `out` / `close`).
    Udp(UdpState),
    /// A child process lifecycle relationship (`in` waits, `out ..` terminates,
    /// `close` releases). Holds the process handle and, once observed, the
    /// stable termination result.
    ChildProc(ChildProcState),
    /// A child process input channel (`out` / `close`).
    ChildInput(ChildIn),
    /// A child process normal-output channel (`in` / `close`).
    ChildOutput(ChildOut),
    /// A child process diagnostic-output channel (`in` / `close`).
    ChildError(ChildErr),
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
            Assoc::Tcp(_) => "TCP connection",
            Assoc::TcpListener(_) => "TCP listener",
            Assoc::Udp(_) => "UDP socket",
            Assoc::ChildProc(_) => "child process",
            Assoc::ChildInput(_) => "child input channel",
            Assoc::ChildOutput(_) => "child output channel",
            Assoc::ChildError(_) => "child error channel",
            Assoc::Closed => "closed relationship",
        }
    }
}

/// Text or binary payload representation, fixed when a relationship is opened.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Repr {
    Text,
    Binary,
}

#[derive(Debug)]
pub struct TcpConn {
    stream: std::net::TcpStream,
    mode: Repr,
    /// Partial trailing UTF-8 bytes for a text-mode stream, carried between reads.
    buf: Vec<u8>,
}

#[derive(Debug)]
pub struct TcpListenerState {
    listener: std::net::TcpListener,
    mode: Repr,
}

#[derive(Debug)]
pub struct UdpState {
    socket: std::net::UdpSocket,
    mode: Repr,
}

/// A spawned child process. The stdin/stdout/stderr handles are moved out into
/// their own channel associations at open time, so this holds only the process
/// handle (for waiting and termination) and the cached final termination result
/// (so repeated `in(child)` yields the same value — Addendum C.19).
#[derive(Debug)]
pub struct ChildProcState {
    child: std::process::Child,
    status: Option<Value>,
}

/// A child process input channel: the child's stdin, plus the fixed
/// representation chosen at open time.
#[derive(Debug)]
pub struct ChildIn {
    stream: std::process::ChildStdin,
    mode: Repr,
}

/// A child process normal-output channel: the child's stdout, its
/// representation, and a buffer for a UTF-8 character split across host reads.
#[derive(Debug)]
pub struct ChildOut {
    stream: std::process::ChildStdout,
    mode: Repr,
    buf: Vec<u8>,
}

/// A child process diagnostic-output channel: the child's stderr, its
/// representation, and a split-character buffer.
#[derive(Debug)]
pub struct ChildErr {
    stream: std::process::ChildStderr,
    mode: Repr,
    buf: Vec<u8>,
}

/// Split the longest valid UTF-8 prefix out of `buf`, leaving incomplete
/// trailing bytes behind. `None` means only an incomplete character is present.
/// Errors on genuinely invalid UTF-8.
pub fn take_valid_utf8(buf: &mut Vec<u8>, line: usize) -> Result<Option<String>> {
    match std::str::from_utf8(buf) {
        Ok(s) => {
            if s.is_empty() {
                return Ok(None);
            }
            let out = s.to_string();
            buf.clear();
            Ok(Some(out))
        }
        Err(e) => {
            if e.error_len().is_some() {
                return Err(SixError::at(line, "input was not valid UTF-8"));
            }
            let valid = e.valid_up_to();
            if valid == 0 {
                return Ok(None);
            }
            let out = String::from_utf8(buf[..valid].to_vec()).unwrap();
            buf.drain(..valid);
            Ok(Some(out))
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

// --- network domain (Addendum B) --------------------------------------------
//
// Network relationships are persistent: a descriptor is established with `open`,
// and the returned runtime-backed Group carries the socket. Descriptors are
// never direct in/out targets.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};

fn parse_repr(items: &[Value], idx: usize, line: usize) -> Result<Repr> {
    match items.get(idx) {
        Some(Value::Text(r)) if r == "text" => Ok(Repr::Text),
        Some(Value::Text(r)) if r == "binary" => Ok(Repr::Binary),
        _ => Err(SixError::at(line, "network descriptor needs a representation: \"text\" or \"binary\"")),
    }
}

fn parse_addr(items: &[Value], idx: usize, line: usize) -> Result<String> {
    match items.get(idx) {
        Some(Value::Text(a)) => Ok(a.clone()),
        _ => Err(SixError::at(line, "network descriptor needs a text address")),
    }
}

fn parse_port(items: &[Value], idx: usize, line: usize) -> Result<u16> {
    match items.get(idx) {
        Some(Value::Number(n)) if n.fract() == 0.0 && *n >= 0.0 && *n <= 65535.0 => Ok(*n as u16),
        _ => Err(SixError::at(line, "network descriptor needs an integer port 0..65535")),
    }
}

/// Establish a network relationship from a `["network" ...]` descriptor.
pub fn net_open(items: &[Value], line: usize) -> Result<Assoc> {
    let proto = match items.get(1) {
        Some(Value::Text(p)) => p.as_str(),
        _ => return Err(SixError::at(line, "network descriptor needs a protocol: \"tcp\" or \"udp\"")),
    };
    match proto {
        "tcp" => {
            let is_listener = matches!(items.get(2), Some(Value::Text(t)) if t == "listener");
            if is_listener {
                let addr = parse_addr(items, 3, line)?;
                let port = parse_port(items, 4, line)?;
                let mode = parse_repr(items, 5, line)?;
                let listener = TcpListener::bind((addr.as_str(), port))
                    .map_err(|e| SixError::at(line, format!("network: cannot listen on {}:{}: {}", addr, port, e)))?;
                Ok(Assoc::TcpListener(TcpListenerState { listener, mode }))
            } else {
                let addr = parse_addr(items, 2, line)?;
                let port = parse_port(items, 3, line)?;
                let mode = parse_repr(items, 4, line)?;
                let stream = TcpStream::connect((addr.as_str(), port))
                    .map_err(|e| SixError::at(line, format!("network: cannot connect to {}:{}: {}", addr, port, e)))?;
                Ok(Assoc::Tcp(TcpConn { stream, mode, buf: Vec::new() }))
            }
        }
        "udp" => {
            let addr = parse_addr(items, 2, line)?;
            let port = parse_port(items, 3, line)?;
            let mode = parse_repr(items, 4, line)?;
            let socket = UdpSocket::bind((addr.as_str(), port))
                .map_err(|e| SixError::at(line, format!("network: cannot bind UDP {}:{}: {}", addr, port, e)))?;
            Ok(Assoc::Udp(UdpState { socket, mode }))
        }
        other => Err(SixError::at(line, format!("unknown network protocol \"{}\"", other))),
    }
}

/// Accept the next incoming connection on a listener (blocks).
pub fn tcp_accept(l: &TcpListenerState, line: usize) -> Result<Assoc> {
    let (stream, _peer) =
        l.listener.accept().map_err(|e| SixError::at(line, format!("network: accept failed: {}", e)))?;
    Ok(Assoc::Tcp(TcpConn { stream, mode: l.mode, buf: Vec::new() }))
}

pub fn tcp_in(c: &mut TcpConn, line: usize) -> Result<Value> {
    match c.mode {
        Repr::Binary => {
            let mut chunk = [0u8; 4096];
            let n = c.stream.read(&mut chunk).map_err(|e| SixError::at(line, format!("network: read failed: {}", e)))?;
            if n == 0 {
                return Ok(Value::Empty); // EOF
            }
            Ok(Value::new_group(chunk[..n].iter().map(|b| byte(*b)).collect()))
        }
        Repr::Text => loop {
            if let Some(text) = take_valid_utf8(&mut c.buf, line)? {
                return Ok(Value::Text(text));
            }
            let mut chunk = [0u8; 4096];
            let n = c.stream.read(&mut chunk).map_err(|e| SixError::at(line, format!("network: read failed: {}", e)))?;
            if n == 0 {
                if c.buf.is_empty() {
                    return Ok(Value::Empty);
                }
                return Err(SixError::at(line, "network: stream ended in the middle of a UTF-8 character"));
            }
            c.buf.extend_from_slice(&chunk[..n]);
        },
    }
}

pub fn tcp_out(c: &mut TcpConn, value: &Value, line: usize) -> Result<Value> {
    let bytes = payload_bytes(value, c.mode, line)?;
    c.stream.write_all(&bytes).map_err(|e| SixError::at(line, format!("network: write failed: {}", e)))?;
    let _ = c.stream.flush();
    Ok(Value::Empty)
}

pub fn udp_in(u: &mut UdpState, line: usize) -> Result<Value> {
    // 65536 covers the maximum UDP payload (65507), so a valid datagram is never
    // truncated.
    let mut buf = vec![0u8; 65536];
    let (n, src) =
        u.socket.recv_from(&mut buf).map_err(|e| SixError::at(line, format!("network: UDP receive failed: {}", e)))?;
    let data = &buf[..n];
    let payload = match u.mode {
        Repr::Binary => Value::new_group(data.iter().map(|b| byte(*b)).collect()),
        Repr::Text => match std::str::from_utf8(data) {
            Ok(s) => Value::Text(s.to_string()),
            Err(_) => return Err(SixError::at(line, "network: UDP datagram was not valid UTF-8")),
        },
    };
    Ok(Value::new_group(vec![Value::Text(src.ip().to_string()), Value::Number(src.port() as f64), payload]))
}

pub fn udp_out(u: &mut UdpState, value: &Value, line: usize) -> Result<Value> {
    let items = match value {
        Value::Group(g) => g.items.borrow().clone(),
        other => {
            return Err(SixError::at(line, format!("UDP out needs a Group [address port data], not a {}", other.type_name())));
        }
    };
    if items.len() != 3 {
        return Err(SixError::at(line, "UDP out needs exactly [address port data]"));
    }
    let addr = match &items[0] {
        Value::Text(a) => a.clone(),
        other => return Err(SixError::at(line, format!("UDP destination address must be text, not a {}", other.type_name()))),
    };
    let port = match &items[1] {
        Value::Number(n) if n.fract() == 0.0 && *n >= 0.0 && *n <= 65535.0 => *n as u16,
        _ => return Err(SixError::at(line, "UDP destination port must be an integer 0..65535")),
    };
    let bytes = payload_bytes(&items[2], u.mode, line)?;
    let sent = u
        .socket
        .send_to(&bytes, (addr.as_str(), port))
        .map_err(|e| SixError::at(line, format!("network: UDP send failed: {}", e)))?;
    if sent != bytes.len() {
        return Err(SixError::at(line, "network: UDP datagram was not fully submitted"));
    }
    Ok(Value::Empty)
}

/// Encode a Six value as bytes according to a relationship's representation.
fn payload_bytes(value: &Value, mode: Repr, line: usize) -> Result<Vec<u8>> {
    match mode {
        Repr::Text => match value {
            Value::Text(s) => Ok(s.as_bytes().to_vec()),
            other => Err(SixError::at(line, format!("this text relationship needs text, not a {}", other.type_name()))),
        },
        Repr::Binary => value_to_bytes(value, line),
    }
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

// --- process domain (Addendum C) --------------------------------------------
//
// Two kinds of target share the `["process" ...]` prefix:
//   * current-process descriptors  — direct, association-free state (arguments,
//     environment, working directory, exit), handled by `in`/`out`;
//   * child-process descriptors     — `["process" program args repr options?]`,
//     established with `open` into a runtime-backed Group whose visible contents
//     are the `input`/`output`/`error` channel Groups.

use std::collections::HashSet;
use std::process::{Command, Stdio};

/// A parsed `["process" ...]` descriptor. Length disambiguates: 1 = exit,
/// 2/3 = a current-process form, 4/5 = a child descriptor (so a child program
/// may even be named "arguments" or "environment").
pub enum ProcDesc {
    Exit,
    Arguments,
    Environment,
    EnvVar(String),
    Directory,
    Child,
}

/// Classify a `["process" ...]` descriptor by shape.
pub fn parse_process(items: &[Value], line: usize) -> Result<ProcDesc> {
    match items.len() {
        1 => Ok(ProcDesc::Exit),
        2 => match &items[1] {
            Value::Text(k) if k == "arguments" => Ok(ProcDesc::Arguments),
            Value::Text(k) if k == "environment" => Ok(ProcDesc::Environment),
            Value::Text(k) if k == "directory" => Ok(ProcDesc::Directory),
            _ => Err(SixError::at(
                line,
                "process descriptor: expected \"arguments\", \"environment\", or \"directory\"",
            )),
        },
        3 => match &items[1] {
            Value::Text(k) if k == "environment" => match &items[2] {
                Value::Text(name) => Ok(ProcDesc::EnvVar(name.clone())),
                _ => Err(SixError::at(line, "process environment name must be text")),
            },
            _ => Err(SixError::at(line, "malformed process descriptor")),
        },
        4 | 5 => Ok(ProcDesc::Child),
        _ => Err(SixError::at(line, "malformed process descriptor")),
    }
}

/// `in(["process" "arguments"])` — the current program's arguments as a Group of
/// Text (`[]` when none). Observational; never writable.
pub fn proc_arguments(args: &[String]) -> Value {
    Value::new_group(args.iter().map(|a| Value::Text(a.clone())).collect())
}

/// `in(["process" "environment"])` — the whole environment as a Group of
/// `[name value]` Text pairs (order unspecified). A name or value that is not
/// valid Six Text is a runtime error.
pub fn proc_env_all(line: usize) -> Result<Value> {
    let mut pairs = Vec::new();
    for (k, v) in std::env::vars_os() {
        let name = k
            .into_string()
            .map_err(|_| SixError::at(line, "process: an environment variable name is not valid text"))?;
        let value = v
            .into_string()
            .map_err(|_| SixError::at(line, format!("process: the value of {} is not valid text", name)))?;
        pairs.push(pair(&name, Value::Text(value)));
    }
    Ok(Value::new_group(pairs))
}

/// `in(["process" "environment" name])` — the variable's value as Text, or `..`
/// when it is not set. A set-but-invalid value is a runtime error.
pub fn proc_env_get(name: &str, line: usize) -> Result<Value> {
    match std::env::var_os(name) {
        None => Ok(Value::Empty),
        Some(v) => match v.into_string() {
            Ok(s) => Ok(Value::Text(s)),
            Err(_) => Err(SixError::at(line, format!("process: the value of {} is not valid text", name))),
        },
    }
}

/// `out(["process" "environment" name] value)` — set (Text) or remove (`..`) a
/// variable; returns `..`.
pub fn proc_env_set(name: &str, value: &Value, line: usize) -> Result<Value> {
    match value {
        Value::Text(v) => std::env::set_var(name, v),
        Value::Empty => std::env::remove_var(name),
        other => {
            return Err(SixError::at(
                line,
                format!("process: an environment value must be text (or .. to remove), not a {}", other.type_name()),
            ));
        }
    }
    Ok(Value::Empty)
}

/// `in(["process" "directory"])` — the current working directory as Text.
pub fn proc_dir_get(line: usize) -> Result<Value> {
    let dir = std::env::current_dir()
        .map_err(|e| SixError::at(line, format!("process: cannot read the working directory: {}", e)))?;
    match dir.into_os_string().into_string() {
        Ok(s) => Ok(Value::Text(s)),
        Err(_) => Err(SixError::at(line, "process: the working directory path is not valid text")),
    }
}

/// `out(["process" "directory"] path)` — change the working directory; `..`.
pub fn proc_dir_set(value: &Value, line: usize) -> Result<Value> {
    let path = match value {
        Value::Text(p) => p,
        other => {
            return Err(SixError::at(line, format!("process: a working directory must be text, not a {}", other.type_name())));
        }
    };
    std::env::set_current_dir(Path::new(path))
        .map_err(|e| SixError::at(line, format!("process: cannot change directory to {}: {}", path, e)))?;
    Ok(Value::Empty)
}

/// `out(["process"] code)` — terminate the current Six process. On success this
/// does not return.
pub fn proc_exit(value: &Value, line: usize) -> Result<Value> {
    let code = match value {
        Value::Number(n) if n.fract() == 0.0 && *n >= 0.0 && *n <= i32::MAX as f64 => *n as i32,
        Value::Number(_) => return Err(SixError::at(line, "process: exit status must be a non-negative integer")),
        other => {
            return Err(SixError::at(line, format!("process: exit status must be a number, not a {}", other.type_name())));
        }
    };
    std::process::exit(code);
}

/// `open(["process" program args repr options?])` — spawn a child and build its
/// runtime-backed Group `[["input" _] ["output" _] ["error" _]]`.
pub fn spawn_child(items: &[Value], line: usize) -> Result<Value> {
    if items.len() < 4 || items.len() > 5 {
        return Err(SixError::at(
            line,
            "open expects a child process descriptor [\"process\" program arguments representation options?]",
        ));
    }
    let program = match &items[1] {
        Value::Text(s) => s.clone(),
        other => return Err(SixError::at(line, format!("process: program must be text, not a {}", other.type_name()))),
    };
    let args = match &items[2] {
        Value::Group(g) => {
            let mut out = Vec::new();
            for a in g.items.borrow().iter() {
                match a {
                    Value::Text(s) => out.push(s.clone()),
                    other => {
                        return Err(SixError::at(line, format!("process: each argument must be text, not a {}", other.type_name())));
                    }
                }
            }
            out
        }
        other => return Err(SixError::at(line, format!("process: arguments must be a Group of text, not a {}", other.type_name()))),
    };
    let mode = parse_repr(items, 3, line)?;

    let mut cmd = Command::new(&program);
    cmd.args(&args);
    if let Some(opt) = items.get(4) {
        apply_child_options(&mut cmd, opt, line)?;
    }
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SixError::at(line, format!("process: program not found: {}", program))
        } else {
            SixError::at(line, format!("process: cannot start {}: {}", program, e))
        }
    })?;
    // With piped stdio these are always present.
    let stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");

    let g_in = crate::value::new_relationship(Vec::new(), Assoc::ChildInput(ChildIn { stream: stdin, mode }));
    let g_out =
        crate::value::new_relationship(Vec::new(), Assoc::ChildOutput(ChildOut { stream: stdout, mode, buf: Vec::new() }));
    let g_err =
        crate::value::new_relationship(Vec::new(), Assoc::ChildError(ChildErr { stream: stderr, mode, buf: Vec::new() }));
    let visible = vec![
        pair("input", Value::Group(g_in)),
        pair("output", Value::Group(g_out)),
        pair("error", Value::Group(g_err)),
    ];
    Ok(Value::Group(crate::value::new_relationship(visible, Assoc::ChildProc(ChildProcState { child, status: None }))))
}

/// Apply the optional final option Group to a child `Command`. The only v1
/// options are `["directory" path]` and `["environment" overrides]`; each must
/// be a two-element keyed Group, names are unique.
fn apply_child_options(cmd: &mut Command, opt: &Value, line: usize) -> Result<()> {
    let opts = match opt {
        Value::Group(g) => g.items.borrow().clone(),
        other => return Err(SixError::at(line, format!("process: options must be a Group, not a {}", other.type_name()))),
    };
    let mut seen: HashSet<String> = HashSet::new();
    for o in &opts {
        let entry = match o {
            Value::Group(g) => g.items.borrow().clone(),
            other => return Err(SixError::at(line, format!("process: each option must be a two-element Group, not a {}", other.type_name()))),
        };
        if entry.len() != 2 {
            return Err(SixError::at(line, "process: each option must be a two-element Group [name value]"));
        }
        let name = match &entry[0] {
            Value::Text(s) => s.clone(),
            other => return Err(SixError::at(line, format!("process: an option name must be text, not a {}", other.type_name()))),
        };
        if !seen.insert(name.clone()) {
            return Err(SixError::at(line, format!("process: duplicate option \"{}\"", name)));
        }
        match name.as_str() {
            "directory" => {
                let path = match &entry[1] {
                    Value::Text(p) => p.clone(),
                    other => return Err(SixError::at(line, format!("process: the directory option needs text, not a {}", other.type_name()))),
                };
                cmd.current_dir(Path::new(&path));
            }
            "environment" => apply_child_env(cmd, &entry[1], line)?,
            other => return Err(SixError::at(line, format!("process: unknown option \"{}\"", other))),
        }
    }
    Ok(())
}

/// Apply an `environment` override Group to a child `Command`. The child starts
/// from a snapshot of the current environment (the default `Command`
/// inheritance); each `[name value]` sets (Text) or removes (`..`) a variable.
fn apply_child_env(cmd: &mut Command, overrides: &Value, line: usize) -> Result<()> {
    let entries = match overrides {
        Value::Group(g) => g.items.borrow().clone(),
        other => return Err(SixError::at(line, format!("process: environment overrides must be a Group, not a {}", other.type_name()))),
    };
    let mut seen: HashSet<String> = HashSet::new();
    for e in &entries {
        let kv = match e {
            Value::Group(g) => g.items.borrow().clone(),
            other => return Err(SixError::at(line, format!("process: each environment override must be a Group [name value], not a {}", other.type_name()))),
        };
        if kv.len() != 2 {
            return Err(SixError::at(line, "process: each environment override must be a Group [name value]"));
        }
        let name = match &kv[0] {
            Value::Text(s) => s.clone(),
            other => return Err(SixError::at(line, format!("process: an environment name must be text, not a {}", other.type_name()))),
        };
        if !seen.insert(name.clone()) {
            return Err(SixError::at(line, format!("process: duplicate environment override \"{}\"", name)));
        }
        match &kv[1] {
            Value::Text(v) => {
                cmd.env(&name, v);
            }
            Value::Empty => {
                cmd.env_remove(&name);
            }
            other => return Err(SixError::at(line, format!("process: an environment value must be text or .., not a {}", other.type_name()))),
        }
    }
    Ok(())
}

/// `in(child)` — block until the child terminates and return its stable result:
/// a non-negative integer exit status, or `..` for signal/abnormal termination.
pub fn child_wait(state: &mut ChildProcState, line: usize) -> Result<Value> {
    if let Some(v) = &state.status {
        return Ok(v.clone());
    }
    let status = state
        .child
        .wait()
        .map_err(|e| SixError::at(line, format!("process: waiting for the child failed: {}", e)))?;
    let result = match status.code() {
        Some(c) if c >= 0 => Value::Number(c as f64),
        _ => Value::Empty,
    };
    state.status = Some(result.clone());
    Ok(result)
}

/// `out(child ..)` — request child termination. Only `..` is accepted; success
/// returns `..` and does not assert the child has already exited.
pub fn child_terminate(state: &mut ChildProcState, value: &Value, line: usize) -> Result<Value> {
    if !value.is_empty() {
        return Err(SixError::at(line, "process: the only output to a child relationship is .. (terminate)"));
    }
    // An already-exited child makes kill() an error on some platforms; that is
    // success here — termination is the goal.
    match state.child.kill() {
        Ok(()) => Ok(Value::Empty),
        Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => Ok(Value::Empty),
        Err(e) => Err(SixError::at(line, format!("process: cannot terminate the child: {}", e))),
    }
}

/// `out(child["input"] value)` — write to the child's stdin; returns `..`.
pub fn child_in_write(c: &mut ChildIn, value: &Value, line: usize) -> Result<Value> {
    let bytes = payload_bytes(value, c.mode, line)?;
    c.stream
        .write_all(&bytes)
        .map_err(|e| SixError::at(line, format!("process: writing to the child failed: {}", e)))?;
    let _ = c.stream.flush();
    Ok(Value::Empty)
}

/// Read one chunk from a child output/error stream according to `mode`
/// (Addendum C.15–C.18): non-empty Text or byte Group, or `..` at EOF.
fn read_child_stream(stream: &mut dyn Read, mode: Repr, buf: &mut Vec<u8>, line: usize) -> Result<Value> {
    match mode {
        Repr::Binary => {
            let mut chunk = [0u8; 4096];
            let n = stream
                .read(&mut chunk)
                .map_err(|e| SixError::at(line, format!("process: reading from the child failed: {}", e)))?;
            if n == 0 {
                return Ok(Value::Empty);
            }
            Ok(Value::new_group(chunk[..n].iter().map(|b| byte(*b)).collect()))
        }
        Repr::Text => loop {
            if let Some(text) = take_valid_utf8(buf, line)? {
                return Ok(Value::Text(text));
            }
            let mut chunk = [0u8; 4096];
            let n = stream
                .read(&mut chunk)
                .map_err(|e| SixError::at(line, format!("process: reading from the child failed: {}", e)))?;
            if n == 0 {
                if buf.is_empty() {
                    return Ok(Value::Empty);
                }
                return Err(SixError::at(line, "process: child output ended in the middle of a UTF-8 character"));
            }
            buf.extend_from_slice(&chunk[..n]);
        },
    }
}

/// `in(child["output"])`.
pub fn child_out_read(c: &mut ChildOut, line: usize) -> Result<Value> {
    read_child_stream(&mut c.stream, c.mode, &mut c.buf, line)
}

/// `in(child["error"])`.
pub fn child_err_read(c: &mut ChildErr, line: usize) -> Result<Value> {
    read_child_stream(&mut c.stream, c.mode, &mut c.buf, line)
}

// --- device domain (Addendum D) ---------------------------------------------
//
// The device domain is an extensible adapter boundary. v1 registers no concrete
// device adapters, so discovery is always empty and opening any device kind is
// an "unknown adapter" runtime error. The dispatch shape is what later adapters
// plug into; nothing here adds Six syntax, primitives, or value types.

/// `in(["device"])` — discover host-exposed devices. With no adapters in v1,
/// this is always the empty Group.
pub fn device_discover() -> Result<Value> {
    Ok(Value::new_group(Vec::new()))
}

/// `open(["device" kind identifier? options?])` — establish a device
/// relationship through the named adapter. No adapters exist in v1, so a
/// well-formed descriptor still fails with "unknown device adapter".
pub fn device_open(items: &[Value], line: usize) -> Result<Value> {
    let kind = match items.get(1) {
        Some(Value::Text(k)) => k.clone(),
        Some(other) => {
            return Err(SixError::at(line, format!("device: kind must be text, not a {}", other.type_name())));
        }
        None => {
            return Err(SixError::at(line, "device: the discovery descriptor [\"device\"] cannot be opened; it is in-only"));
        }
    };
    Err(SixError::at(
        line,
        format!("device: unknown device adapter \"{}\" (no device adapters are available in this build)", kind),
    ))
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
