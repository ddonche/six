//! The Six prelude.
//!
//! Six v0.1 loads no standard-library functions into global scope. The
//! higher-order helpers the specification illustrates (`map`, `filter`, `fold`,
//! `find`) are *user-written* in Six — the canonical `tiny_inventory` program,
//! for instance, defines its own `find`, so preloading a `find` would collide
//! with it. The runtime names — the six builtins (`print`, `input`, `size`,
//! `insert`, `remove`, `has`) and the two conversions (`number`, `text`) — are
//! registered directly by the interpreter, not through this prelude.
pub const PRELUDE: &str = "";
