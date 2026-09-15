//! The Six prelude: standard-library operations implemented *in Six*.
//!
//! Per spec §38, library operations should be written in Six wherever
//! practical. These four higher-order functions are, which both keeps the Rust
//! core small and demonstrates that the language bootstraps them from Groups,
//! recursion, and guaranteed tail calls alone.

pub const PRELUDE: &str = r#"
>> fold: [list init f] >>
    _fold list init f 0
.

>> _fold: [list acc f i] >>
    if
        i >= size list >> acc
        else           >> _fold list (f acc (list[i])) f (i + 1)
    .
.

>> map: [list f] >>
    _map list f 0 []
.

>> _map: [list f i result] >>
    if
        i >= size list >> result
        else >>
            insert result (f (list[i]))
            _map list f (i + 1) result
    .
.

>> filter: [list f] >>
    _filter list f 0 []
.

>> _filter: [list f i result] >>
    if
        i >= size list >> result
        else >>
            if f (list[i]) >>
                insert result (list[i])
            .
            _filter list f (i + 1) result
    .
.

>> find: [list f] >>
    _find list f 0
.

>> _find: [list f i] >>
    if
        i >= size list >> ..
        f (list[i])    >> list[i]
        else           >> _find list f (i + 1)
    .
.
"#;
