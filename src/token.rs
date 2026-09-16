//! Token definitions for the Six lexer.

/// The lexical categories Six recognises.
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    // --- literals ---
    Number(f64),
    Text(String),
    True,
    False,
    Nil,   // `nil`
    Empty, // `..`
    Ident(String),
    Dollar, // `$` — the final valid index

    // --- binding / assignment ---
    Colon,      // `:`   new binding
    ColonColon, // `::`  deep-copy binding
    Assign,     // `=`   rebind

    // --- grouping ---
    LBracket, // `[`
    RBracket, // `]`
    LParen,   // `(`
    RParen,   // `)`

    // --- comparison ---
    Lt, // `<`
    Gt, // `>`
    Le, // `<=`
    Ge, // `>=`
    EqEq,  // `==`
    NotEq, // `!=`

    // --- arithmetic ---
    Plus,
    Minus,
    Star,
    Slash,
    Percent,

    // --- postfix operators (hug the operand, no space before) ---
    Tilde,      // `~`  round
    Caret,      // `^`  ceiling
    Underscore, // `_`  floor
    StarStar,   // `**` square / power
    SlashSlash, // `//` square root
    PlusPlus,   // `++` increment
    MinusMinus, // `--` decrement

    // --- logical ---
    And, // `and` / `&&`
    Or,  // `or`  / `||`
    Not, // `not` / `!`

    // --- flow / declaration ---
    FatArrow, // `>>`

    // --- dot ---
    Dot, // `.` (block terminator OR dot-flow, decided by the parser)

    // --- conditionals ---
    If,   // `if`
    Any,  // `any`
    Else, // `else`
    QIf,  // `?`
    QElse,// `??`
    QAny, // `?*`

    // --- structural ---
    Newline,
    Eof,
}

/// A token plus the source metadata Six's whitespace-sensitive grammar needs.
#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
    pub col: usize,
    /// Whether whitespace (or a newline, or start-of-input) preceded this token.
    /// This is how the parser distinguishes `arr[0]` (index) from `f [1 2 3]`
    /// (a Group argument), and `x.size` (dot-flow) from a standalone `.`
    /// (block terminator).
    pub space_before: bool,
}

impl Token {
    pub fn new(tok: Tok, line: usize, col: usize, space_before: bool) -> Self {
        Token { tok, line, col, space_before }
    }
}
