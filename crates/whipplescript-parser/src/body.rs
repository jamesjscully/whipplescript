//! Rule and flow body parsing: a real AST over body text.
//!
//! Bodies were historically re-scanned line-by-line at lowering time, which
//! made whitespace load-bearing and let unknown statement forms slip through
//! silently. This module is the statement-form gate: every body must parse
//! into [`BodyAst`], unknown tokens are spanned errors, and lowering consumes
//! structure instead of strings.

use crate::{parse_expression, Diagnostic, Expr, SourceSpan};

/// Parses short durations: `<integer><unit>` with unit `s`, `m`, `h`, or `d`.
pub fn parse_short_duration_seconds(value: &str) -> Option<u64> {
    let unit = value.chars().last()?;
    let number = value.get(..value.len() - 1)?.parse::<u64>().ok()?;
    let multiplier = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86400,
        _ => return None,
    };
    number.checked_mul(multiplier)
}

/// Structural ISO-8601 instant check (`YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)`)
/// for `time` literals, with calendar-field range validation. Kept
/// dependency-free: the runtime compares instants via SQLite `strftime`.
pub fn is_iso8601_instant(value: &str) -> bool {
    let bytes = value.as_bytes();
    let digits = |range: std::ops::Range<usize>| {
        bytes
            .get(range)
            .is_some_and(|slice| !slice.is_empty() && slice.iter().all(u8::is_ascii_digit))
    };
    let field = |range: std::ops::Range<usize>| -> u32 {
        value
            .get(range)
            .and_then(|text| text.parse().ok())
            .unwrap_or(u32::MAX)
    };
    if !(digits(0..4) && bytes.get(4) == Some(&b'-') && digits(5..7))
        || bytes.get(7) != Some(&b'-')
        || !digits(8..10)
        || bytes.get(10) != Some(&b'T')
        || !digits(11..13)
        || bytes.get(13) != Some(&b':')
        || !digits(14..16)
        || bytes.get(16) != Some(&b':')
        || !digits(17..19)
    {
        return false;
    }
    if !(1..=12).contains(&field(5..7))
        || !(1..=31).contains(&field(8..10))
        || field(11..13) > 23
        || field(14..16) > 59
        || field(17..19) > 60
    {
        return false;
    }
    let mut index = 19;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == start {
            return false;
        }
    }
    match bytes.get(index) {
        Some(b'Z') => index + 1 == bytes.len(),
        Some(b'+') | Some(b'-') => {
            digits(index + 1..index + 3)
                && bytes.get(index + 3) == Some(&b':')
                && digits(index + 4..index + 6)
                && index + 6 == bytes.len()
                && field(index + 1..index + 3) <= 23
                && field(index + 4..index + 6) <= 59
        }
        _ => false,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BodyAst {
    pub statements: Vec<BodyStmt>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BodyStmt {
    Record(RecordStmt),
    /// `done x` / `done x -> record ...`; `consume` sets `deprecated_consume`.
    Done {
        binding: String,
        replacement: Option<RecordStmt>,
        deprecated_consume: bool,
        span: SourceSpan,
    },
    Effect(EffectStmt),
    After(AfterBlock),
    Case(CaseBlock),
    /// Flow-only deterministic branch: `when <expr> { ... } [else { ... }]`.
    Branch(BranchBlock),
    /// Flow-only failure handler attached to the preceding effect step.
    Handler(HandlerBlock),
    Terminal(TerminalStmt),
    Cancel {
        binding: String,
        span: SourceSpan,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordStmt {
    pub schema: String,
    pub from: Option<String>,
    pub fields: Vec<FieldAssign>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldAssign {
    pub name: String,
    pub value: FieldValue,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FieldValue {
    /// Bare field in a `from` block: copy the same-named field.
    Shorthand,
    /// An expression, kept with its exact source text for template
    /// rendering and lowering compatibility.
    Expr { source: String, expr: Expr },
    /// Nested typed payload, e.g. invoke input: `phase PhaseReview { ... }`.
    Nested {
        schema: String,
        fields: Vec<FieldAssign>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectStmt {
    pub kind: BodyEffectKind,
    pub binding: Option<String>,
    pub requires: Vec<String>,
    /// `timeout <duration>` in seconds, creation-anchored.
    pub timeout_seconds: Option<u64>,
    pub prompt: Option<Prompt>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BodyEffectKind {
    Tell {
        target: String,
    },
    Coerce {
        name: String,
        args: Vec<String>,
    },
    AskHuman {
        choices: Vec<String>,
    },
    /// Inline anonymous coercion: `decide "<prompt>" -> { field type, ... } as x`.
    Decide {
        result_fields: Vec<(String, String)>,
    },
    Call {
        capability: String,
        argument: Option<String>,
    },
    Invoke {
        workflow: String,
        payload: Vec<FieldAssign>,
    },
    Timer {
        duration_seconds: u64,
        duration_source: String,
        /// Absolute deadline expression (a time literal or a time-typed
        /// path); `None` for a relative `timer <duration>`.
        until: Option<String>,
    },
    Exec {
        target: ExecTarget,
        /// `-> Schema` / `-> each Schema`: deterministic JSON ingestion of
        /// stdout at the effect-result boundary (spec/json-ingestion.md).
        parse_target: Option<ExecParse>,
    },
    /// Work-queue verbs (`file item into q { ... }`, `claim x`, `release x`,
    /// `finish x [{ ... }]`).
    QueueFile {
        queue: String,
        fields: Vec<FieldAssign>,
    },
    QueueClaim {
        item: String,
        legacy_plugin: Option<String>,
    },
    QueueRelease {
        item: String,
    },
    QueueFinish {
        item: String,
        fields: Vec<FieldAssign>,
    },
    /// Coordination verbs (spec/coordination.md): one atomic attempt each,
    /// with branchable sum-typed outcomes.
    LeaseAcquire {
        resource: String,
        key_expr: String,
        /// `until ttl`: fire-and-forget; TTL is the sole release.
        until_ttl: bool,
    },
    LedgerAppend {
        ledger: String,
        schema: String,
        fields: Vec<FieldAssign>,
    },
    CounterConsume {
        counter: String,
        key_expr: String,
        amount_expr: String,
    },
    /// `notify <instance-expr> event <name> { payload }`: inject a typed,
    /// durable event into a known peer instance — directed fire-and-forget
    /// (spec/event-ingress.md, spec/coordination.md messaging).
    Notify {
        target_expr: String,
        event: String,
        fields: Vec<FieldAssign>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecTarget {
    RawCommand(String),
    Capability { name: String, stdin_binding: String },
}

/// The `->` ingestion contract on an `exec`: stdout must parse as `schema`
/// (one object) or, with `each`, as a JSONL/array stream of `schema`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecParse {
    pub schema: String,
    pub each: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prompt {
    pub text: String,
    pub content_type: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AfterBlock {
    pub binding: String,
    pub predicate: AfterPredicate,
    pub alias: Option<String>,
    pub body: Vec<BodyStmt>,
    pub span: SourceSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AfterPredicate {
    Succeeds,
    Fails,
    Completes,
    /// Coordination outcomes (spec/coordination.md): the effect completed
    /// and its sum-typed value carries the matching `variant`.
    Held,
    Contended,
    Ok,
    Over,
}

impl AfterPredicate {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Succeeds => "succeeds",
            Self::Fails => "fails",
            Self::Completes => "completes",
            Self::Held => "held",
            Self::Contended => "contended",
            Self::Ok => "ok",
            Self::Over => "over",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaseBlock {
    pub scrutinee: String,
    pub branches: Vec<CaseBranch>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaseBranch {
    pub pattern: String,
    pub binding: Option<String>,
    pub guard: Option<String>,
    pub body: Vec<BodyStmt>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchBlock {
    pub condition_source: String,
    pub condition: Expr,
    pub then_body: Vec<BodyStmt>,
    pub else_body: Option<Vec<BodyStmt>>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HandlerBlock {
    pub kind: HandlerKind,
    pub body: Vec<BodyStmt>,
    pub span: SourceSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandlerKind {
    OnFails,
    OnTimeout,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalStmt {
    pub kind: TerminalKind,
    pub name: String,
    pub fields: Vec<FieldAssign>,
    pub span: SourceSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalKind {
    Complete,
    Fail,
}

/// Whether flow-only statements (`when/else`, `on fails`, `on timeout`) are
/// permitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BodyMode {
    Rule,
    Flow,
}

/// A field assignment extracted from a record/payload body without braces.
/// `value` is `None` for shorthand-copy fields; otherwise it is the exact
/// source text of the value expression.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SplitFieldAssignment {
    pub name: String,
    pub value: Option<String>,
}

/// Token-level field splitting for record/terminal/table-row bodies. The
/// structure comes from tokens, never from line breaks, so single-line and
/// multi-line blocks behave identically. Shorthand (bare name, `from` blocks
/// only at the call site) is line-delimited: a name with no same-line value
/// is shorthand.
pub fn split_field_assignments(source: &str) -> Vec<SplitFieldAssignment> {
    let mut diagnostics = Vec::new();
    let tokens = lex_body(source, 0, &mut diagnostics);
    let mut parser = BodyParser {
        source,
        base: 0,
        tokens,
        pos: 0,
        mode: BodyMode::Rule,
        diagnostics,
    };
    let mut assignments = Vec::new();
    while let Some(token) = parser.peek() {
        let name_line = token.line;
        let Tok::Ident(name) = token.tok.clone() else {
            parser.pos += 1;
            continue;
        };
        parser.pos += 1;
        let is_shorthand = match parser.peek() {
            None => true,
            Some(next) => next.line != name_line,
        };
        if is_shorthand {
            assignments.push(SplitFieldAssignment { name, value: None });
            continue;
        }
        let value_start = parser.pos;
        if !parser.consume_value_atom() {
            parser.pos += 1;
            continue;
        }
        loop {
            match parser.peek().map(|t| t.tok.clone()) {
                Some(Tok::Op(_)) | Some(Tok::Sym('+')) | Some(Tok::Sym('-'))
                | Some(Tok::Sym('*')) | Some(Tok::Sym('/')) | Some(Tok::Sym('<'))
                | Some(Tok::Sym('>')) => {
                    parser.pos += 1;
                    if !parser.consume_value_atom() {
                        break;
                    }
                }
                Some(Tok::Ident(word)) if word == "and" || word == "or" || word == "in" => {
                    parser.pos += 1;
                    if !parser.consume_value_atom() {
                        break;
                    }
                }
                Some(Tok::Sym('[')) => {
                    parser.consume_balanced('[', ']');
                }
                // A brace body after a value atom is a nested payload —
                // variant construction `Approved { score 0.9 }`
                // (spec/sum-types.md) — captured whole, not flattened.
                Some(Tok::Sym('{')) => {
                    parser.consume_balanced('{', '}');
                    break;
                }
                _ => break,
            }
        }
        let first = &parser.tokens[value_start];
        let last = &parser.tokens[parser.pos - 1];
        assignments.push(SplitFieldAssignment {
            name,
            value: Some(source[first.start..last.end].to_owned()),
        });
    }
    assignments
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    TripleStr {
        text: String,
        content_type: Option<String>,
    },
    Number(String),
    Sym(char),
    Arrow,    // ->
    FatArrow, // =>
    Op(&'static str),
}

#[derive(Clone, Debug)]
struct Token {
    tok: Tok,
    start: usize,
    end: usize,
    line: usize,
}

fn line_of(source: &str, offset: usize) -> usize {
    source[..offset].bytes().filter(|b| *b == b'\n').count()
}

fn lex_body(source: &str, base: usize, diagnostics: &mut Vec<Diagnostic>) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if source[i..].starts_with("\"\"\"") {
            // Triple-quoted prompt with optional content-type on the opener.
            let opener_end = source[i + 3..]
                .find('\n')
                .map(|offset| i + 3 + offset)
                .unwrap_or(source.len());
            let annotation = source[i + 3..opener_end].trim();
            let content_type = (!annotation.is_empty()).then(|| annotation.to_owned());
            let Some(close) = source[opener_end..].find("\"\"\"").map(|o| opener_end + o) else {
                diagnostics.push(Diagnostic {
                    span: SourceSpan {
                        start: base + start,
                        end: base + source.len(),
                    },
                    message: "unterminated multiline string".to_owned(),
                    suggestion: Some("close the prompt with `\"\"\"`".to_owned()),
                });
                break;
            };
            let raw = &source[opener_end..close];
            let text = dedent_prompt(raw);
            tokens.push(Token {
                tok: Tok::TripleStr { text, content_type },
                start,
                end: close + 3,
                line: line_of(source, start),
            });
            i = close + 3;
            continue;
        }
        if c == '"' {
            let mut j = i + 1;
            let mut value = String::new();
            let mut closed = false;
            while j < bytes.len() {
                let cj = bytes[j] as char;
                if cj == '\\' && j + 1 < bytes.len() {
                    value.push(bytes[j + 1] as char);
                    j += 2;
                    continue;
                }
                if cj == '"' {
                    closed = true;
                    break;
                }
                if cj == '\n' {
                    break;
                }
                value.push(cj);
                j += 1;
            }
            if !closed {
                diagnostics.push(Diagnostic {
                    span: SourceSpan {
                        start: base + start,
                        end: base + j,
                    },
                    message: "unterminated string".to_owned(),
                    suggestion: Some("close the string with `\"`".to_owned()),
                });
                i = j;
                continue;
            }
            tokens.push(Token {
                tok: Tok::Str(value),
                start,
                end: j + 1,
                line: line_of(source, start),
            });
            i = j + 1;
            continue;
        }
        if c.is_ascii_digit()
            || (c == '-'
                && bytes
                    .get(i + 1)
                    .is_some_and(|b| (*b as char).is_ascii_digit()))
        {
            let mut j = i + 1;
            while j < bytes.len() {
                let cj = bytes[j] as char;
                if cj.is_ascii_alphanumeric() || cj == '.' || cj == '_' {
                    j += 1;
                } else {
                    break;
                }
            }
            tokens.push(Token {
                tok: Tok::Number(source[i..j].to_owned()),
                start,
                end: j,
                line: line_of(source, start),
            });
            i = j;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let mut j = i + 1;
            while j < bytes.len() {
                let cj = bytes[j] as char;
                if cj.is_ascii_alphanumeric() || cj == '_' || cj == '.' {
                    j += 1;
                } else {
                    break;
                }
            }
            // Trailing dots belong to punctuation, not identifiers.
            let mut end = j;
            while end > i && bytes[end - 1] as char == '.' {
                end -= 1;
            }
            tokens.push(Token {
                tok: Tok::Ident(source[i..end].to_owned()),
                start,
                end,
                line: line_of(source, start),
            });
            i = end.max(i + 1);
            continue;
        }
        if source[i..].starts_with("->") {
            tokens.push(Token {
                tok: Tok::Arrow,
                start,
                end: i + 2,
                line: line_of(source, i),
            });
            i += 2;
            continue;
        }
        if source[i..].starts_with("=>") {
            tokens.push(Token {
                tok: Tok::FatArrow,
                start,
                end: i + 2,
                line: line_of(source, i),
            });
            i += 2;
            continue;
        }
        let two_char = [
            ("==", "=="),
            ("!=", "!="),
            ("<=", "<="),
            (">=", ">="),
            ("&&", "&&"),
            ("||", "||"),
        ]
        .iter()
        .find(|(text, _)| source[i..].starts_with(text))
        .map(|(_, op)| *op);
        if let Some(op) = two_char {
            tokens.push(Token {
                tok: Tok::Op(op),
                start,
                end: i + 2,
                line: line_of(source, i),
            });
            i += 2;
            continue;
        }
        match c {
            '{' | '}' | '[' | ']' | '(' | ')' | ',' | '.' | '+' | '-' | '*' | '/' | '<' | '>'
            | '!' | ':' | ';' => {
                tokens.push(Token {
                    tok: Tok::Sym(c),
                    start,
                    end: i + 1,
                    line: line_of(source, start),
                });
                i += 1;
            }
            _ => {
                diagnostics.push(Diagnostic {
                    span: SourceSpan {
                        start: base + i,
                        end: base + i + 1,
                    },
                    message: format!("unexpected character `{c}` in rule body"),
                    suggestion: None,
                });
                i += 1;
            }
        }
    }
    tokens
}

fn dedent_prompt(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let indent = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut text = lines
        .iter()
        .map(|line| {
            if line.len() >= indent {
                &line[indent..]
            } else {
                line.trim_start()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    while text.starts_with('\n') {
        text.remove(0);
    }
    while text.ends_with('\n') || text.ends_with(' ') {
        text.pop();
    }
    text
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

pub fn parse_rule_body(source: &str, base: usize, mode: BodyMode) -> (BodyAst, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let tokens = lex_body(source, base, &mut diagnostics);
    let mut parser = BodyParser {
        source,
        base,
        tokens,
        pos: 0,
        mode,
        diagnostics,
    };
    let statements = parser.parse_statements(false);
    (BodyAst { statements }, parser.diagnostics)
}

struct BodyParser<'a> {
    source: &'a str,
    base: usize,
    tokens: Vec<Token>,
    pos: usize,
    mode: BodyMode,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> BodyParser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset)
    }

    fn advance(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    fn at_ident(&self, value: &str) -> bool {
        matches!(self.peek().map(|t| &t.tok), Some(Tok::Ident(v)) if v == value)
    }

    fn at_sym(&self, value: char) -> bool {
        matches!(self.peek().map(|t| &t.tok), Some(Tok::Sym(v)) if *v == value)
    }

    fn consume_ident(&mut self, value: &str) -> bool {
        if self.at_ident(value) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn consume_sym(&mut self, value: char) -> bool {
        if self.at_sym(value) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn span_here(&self) -> SourceSpan {
        match self.peek() {
            Some(token) => SourceSpan {
                start: self.base + token.start,
                end: self.base + token.end,
            },
            None => SourceSpan {
                start: self.base + self.source.len(),
                end: self.base + self.source.len(),
            },
        }
    }

    fn span_from(&self, start_token: usize) -> SourceSpan {
        let start = self
            .tokens
            .get(start_token)
            .map(|t| self.base + t.start)
            .unwrap_or(self.base);
        let end = self
            .tokens
            .get(self.pos.saturating_sub(1))
            .map(|t| self.base + t.end)
            .unwrap_or(start);
        SourceSpan { start, end }
    }

    fn error(&mut self, span: SourceSpan, message: impl Into<String>, suggestion: Option<String>) {
        self.diagnostics.push(Diagnostic {
            span,
            message: message.into(),
            suggestion,
        });
    }

    fn ident_text(&mut self, what: &str) -> Option<String> {
        match self.peek().map(|t| t.tok.clone()) {
            Some(Tok::Ident(value)) => {
                self.pos += 1;
                Some(value)
            }
            _ => {
                let span = self.span_here();
                self.error(span, format!("expected {what}"), None);
                None
            }
        }
    }

    /// Skip to a safe resync point after an error: the next statement keyword
    /// at the current depth or a closing brace.
    fn recover(&mut self) {
        let mut depth = 0usize;
        while let Some(token) = self.peek() {
            match &token.tok {
                Tok::Sym('{') => depth += 1,
                Tok::Sym('}') if depth == 0 => return,
                Tok::Sym('}') => depth -= 1,
                Tok::Ident(value)
                    if depth == 0
                        && STATEMENT_KEYWORDS.contains(&value.as_str())
                        && self.pos != 0 =>
                {
                    return
                }
                _ => {}
            }
            self.pos += 1;
        }
    }

    fn parse_statements(&mut self, in_block: bool) -> Vec<BodyStmt> {
        let mut statements = Vec::new();
        loop {
            if self.peek().is_none() {
                if in_block {
                    let span = self.span_here();
                    self.error(
                        span,
                        "unclosed block in rule body",
                        Some("add `}`".to_owned()),
                    );
                }
                return statements;
            }
            if self.at_sym('}') {
                if in_block {
                    self.pos += 1;
                }
                return statements;
            }
            let before = self.pos;
            if let Some(statement) = self.parse_statement() {
                statements.push(statement);
            }
            if self.pos == before {
                // No progress: recover to avoid an infinite loop.
                self.pos += 1;
                self.recover();
            }
        }
    }

    fn parse_statement(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        let keyword = match self.peek().map(|t| t.tok.clone()) {
            Some(Tok::Ident(value)) => value,
            _ => {
                let span = self.span_here();
                self.error(
                    span,
                    "expected a rule body statement".to_owned(),
                    Some(
                        "statements start with record, done, tell, coerce, askHuman, claim, \
                         release, finish, file, call, invoke, after, case, complete, fail, \
                         timer, cancel, decide, or exec"
                            .to_owned(),
                    ),
                );
                self.recover();
                return None;
            }
        };
        match keyword.as_str() {
            "record" => self.parse_record_statement().map(BodyStmt::Record),
            // `consume <counter> for <key> ...` is the counter verb
            // (spec/coordination.md); bare `consume <binding>` stays the
            // deprecated done-alias.
            "consume" if self.looks_like_counter_consume() => self.parse_counter_consume(),
            "done" | "consume" => self.parse_done_statement(),
            "tell" => self.parse_tell(),
            "coerce" => self.parse_coerce_call(),
            "askHuman" => self.parse_ask_human(),
            "decide" => self.parse_decide(),
            "call" => self.parse_call(),
            "invoke" => self.parse_invoke(),
            "after" => self.parse_after(),
            "case" => self.parse_case(),
            "complete" | "fail" => self.parse_terminal(),
            "timer" => self.parse_timer(),
            "cancel" => self.parse_cancel(),
            "exec" => self.parse_exec(),
            "file" => self.parse_queue_file(),
            "claim" => self.parse_queue_claim(),
            "release" => self.parse_queue_release(),
            "finish" => self.parse_queue_finish(),
            "acquire" => self.parse_lease_acquire(),
            "append" => self.parse_ledger_append(),
            "notify" => self.parse_notify(),
            "when" if self.mode == BodyMode::Flow => self.parse_branch(),
            "on" if self.mode == BodyMode::Flow => self.parse_handler(),
            "when" | "on" => {
                let span = self.span_here();
                self.error(
                    span,
                    format!("`{keyword}` blocks are only valid inside `flow` bodies"),
                    Some("use a guarded rule, or move this into a flow".to_owned()),
                );
                self.pos += 1;
                self.recover();
                None
            }
            "emit" => {
                let span = self.span_here();
                self.error(
                    span,
                    "`emit` was removed from the language".to_owned(),
                    Some("events are appended by the runtime; record a fact instead".to_owned()),
                );
                self.pos += 1;
                self.recover();
                None
            }
            other => {
                let span = self.span_here();
                self.error(
                    span,
                    format!("unknown rule body statement `{other}`"),
                    Some(
                        "statements start with record, done, tell, coerce, askHuman, claim, \
                         release, finish, file, call, invoke, after, case, complete, fail, \
                         timer, cancel, decide, or exec"
                            .to_owned(),
                    ),
                );
                self.pos += 1;
                self.recover();
                None
            }
        }
        .map(|statement| {
            let _ = start;
            statement
        })
    }

    // -- record ------------------------------------------------------------

    fn parse_record_statement(&mut self) -> Option<RecordStmt> {
        let start = self.pos;
        self.pos += 1; // record
        let schema = self.ident_text("class name after `record`")?;
        let from = if self.consume_ident("from") {
            Some(self.ident_text("binding name after `from`")?)
        } else {
            None
        };
        let fields = self.parse_field_block(from.is_some())?;
        Some(RecordStmt {
            schema,
            from,
            fields,
            span: self.span_from(start),
        })
    }

    fn parse_done_statement(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        let keyword = match self.advance()?.tok {
            Tok::Ident(value) => value,
            _ => return None,
        };
        let binding = self.ident_text("fact binding after `done`")?;
        let replacement = if matches!(self.peek().map(|t| &t.tok), Some(Tok::Arrow)) {
            self.pos += 1;
            if !self.consume_ident("record") {
                let span = self.span_here();
                self.error(span, "expected `record` after `->`", None);
                return None;
            }
            self.pos -= 1; // parse_record_statement expects to consume `record`
            Some(self.parse_record_statement()?)
        } else {
            None
        };
        Some(BodyStmt::Done {
            binding,
            replacement,
            deprecated_consume: keyword == "consume",
            span: self.span_from(start),
        })
    }

    /// Parse `{ field value ... }`. Values are expressions; in `from` blocks a
    /// bare field name is shorthand-copy. Single-line and multi-line forms are
    /// equivalent: structure comes from tokens, never line breaks.
    fn parse_field_block(&mut self, allow_shorthand: bool) -> Option<Vec<FieldAssign>> {
        if !self.consume_sym('{') {
            let span = self.span_here();
            self.error(span, "expected `{` to open a field block", None);
            return None;
        }
        let mut fields = Vec::new();
        loop {
            if self.consume_sym('}') {
                return Some(fields);
            }
            if self.peek().is_none() {
                let span = self.span_here();
                self.error(span, "unclosed field block", Some("add `}`".to_owned()));
                return Some(fields);
            }
            let field_start = self.pos;
            let Some(name) = self.ident_text("field name") else {
                self.recover();
                continue;
            };
            // Nested typed payload: `binding Schema { ... }`.
            if matches!(self.peek().map(|t| &t.tok), Some(Tok::Ident(next))
                if next.chars().next().is_some_and(char::is_uppercase))
                && matches!(self.peek_at(1).map(|t| &t.tok), Some(Tok::Sym('{')))
            {
                let schema = self.ident_text("payload class name")?;
                let nested = self.parse_field_block(false)?;
                fields.push(FieldAssign {
                    name,
                    value: FieldValue::Nested {
                        schema,
                        fields: nested,
                    },
                    span: self.span_from(field_start),
                });
                continue;
            }
            // `from` blocks support shorthand: a bare field name copies the
            // same-named field. Shorthand is line-delimited (the historical
            // and documented form): a name is shorthand when the next token
            // sits on a different line or closes the block.
            if allow_shorthand {
                let name_line = self
                    .tokens
                    .get(field_start)
                    .map(|t| t.line)
                    .unwrap_or_default();
                let is_shorthand = match self.peek() {
                    None => true,
                    Some(token) => matches!(token.tok, Tok::Sym('}')) || token.line != name_line,
                };
                if is_shorthand {
                    fields.push(FieldAssign {
                        name,
                        value: FieldValue::Shorthand,
                        span: self.span_from(field_start),
                    });
                    continue;
                }
            }
            let Some((source, expr)) = self.parse_value_expression() else {
                self.recover();
                continue;
            };
            fields.push(FieldAssign {
                name,
                value: FieldValue::Expr { source, expr },
                span: self.span_from(field_start),
            });
        }
    }

    /// Capture one expression's source slice by walking atoms and operators,
    /// then parse it with the shared expression parser.
    fn parse_value_expression(&mut self) -> Option<(String, Expr)> {
        let start_token = self.pos;
        if !self.consume_value_atom() {
            let span = self.span_here();
            self.error(span, "expected a field value expression", None);
            return None;
        }
        loop {
            match self.peek().map(|t| t.tok.clone()) {
                Some(Tok::Op(_)) | Some(Tok::Sym('+')) | Some(Tok::Sym('-'))
                | Some(Tok::Sym('*')) | Some(Tok::Sym('/')) | Some(Tok::Sym('<'))
                | Some(Tok::Sym('>')) => {
                    self.pos += 1;
                    if !self.consume_value_atom() {
                        let span = self.span_here();
                        self.error(span, "expected expression after operator", None);
                        return None;
                    }
                }
                Some(Tok::Ident(word)) if word == "and" || word == "or" || word == "in" => {
                    self.pos += 1;
                    if !self.consume_value_atom() {
                        let span = self.span_here();
                        self.error(span, "expected expression after operator", None);
                        return None;
                    }
                }
                Some(Tok::Sym('[')) => {
                    // index continuation
                    self.consume_balanced('[', ']');
                }
                _ => break,
            }
        }
        let first = self.tokens.get(start_token)?;
        let last = self.tokens.get(self.pos.saturating_sub(1))?;
        let source = self.source[first.start..last.end].to_owned();
        match parse_expression(&source) {
            Ok(expr) => Some((source, expr)),
            Err(message) => {
                let span = SourceSpan {
                    start: self.base + first.start,
                    end: self.base + last.end,
                };
                self.error(
                    span,
                    format!("invalid field value expression: {message}"),
                    None,
                );
                None
            }
        }
    }

    fn consume_value_atom(&mut self) -> bool {
        match self.peek().map(|t| t.tok.clone()) {
            Some(Tok::Str(_)) | Some(Tok::Number(_)) | Some(Tok::TripleStr { .. }) => {
                self.pos += 1;
                true
            }
            Some(Tok::Sym('[')) => self.consume_balanced('[', ']'),
            Some(Tok::Sym('{')) => self.consume_balanced('{', '}'),
            Some(Tok::Sym('(')) => self.consume_balanced('(', ')'),
            Some(Tok::Sym('!')) | Some(Tok::Sym('-')) => {
                self.pos += 1;
                self.consume_value_atom()
            }
            Some(Tok::Ident(word)) if word == "not" => {
                self.pos += 1;
                self.consume_value_atom()
            }
            Some(Tok::Ident(_)) => {
                self.pos += 1;
                // call like count(...) / exists(...)
                if self.at_sym('(') {
                    self.consume_balanced('(', ')');
                }
                true
            }
            _ => false,
        }
    }

    fn consume_balanced(&mut self, open: char, close: char) -> bool {
        if !self.consume_sym(open) {
            return false;
        }
        let mut depth = 1;
        while depth > 0 {
            match self.advance().map(|t| t.tok) {
                Some(Tok::Sym(c)) if c == open => depth += 1,
                Some(Tok::Sym(c)) if c == close => depth -= 1,
                Some(_) => {}
                None => {
                    let span = self.span_here();
                    self.error(span, format!("unclosed `{open}`"), None);
                    return false;
                }
            }
        }
        true
    }

    // -- effects -----------------------------------------------------------

    fn parse_effect_modifiers(
        &mut self,
        binding: &mut Option<String>,
        requires: &mut Vec<String>,
        timeout_seconds: &mut Option<u64>,
        choices: Option<&mut Vec<String>>,
    ) -> bool {
        let mut choices = choices;
        loop {
            if self.consume_ident("as") {
                match self.ident_text("binding name after `as`") {
                    Some(name) => *binding = Some(name),
                    None => return false,
                }
                continue;
            }
            if self.consume_ident("requires") {
                match self.parse_string_array() {
                    Some(values) => *requires = values,
                    None => return false,
                }
                continue;
            }
            if self.consume_ident("timeout") {
                let span = self.span_here();
                let Some(Tok::Number(value)) = self.peek().map(|t| t.tok.clone()) else {
                    self.error(
                        span,
                        "expected a duration after `timeout`".to_owned(),
                        Some(
                            "use `<n><unit>` with unit s, m, h, or d, e.g. `timeout 10m`"
                                .to_owned(),
                        ),
                    );
                    return false;
                };
                self.pos += 1;
                match parse_short_duration_seconds(&value) {
                    Some(seconds) if seconds > 0 => *timeout_seconds = Some(seconds),
                    _ => {
                        self.error(
                            span,
                            format!("invalid timeout duration `{value}`"),
                            Some("use `<n><unit>` with unit s, m, h, or d".to_owned()),
                        );
                        return false;
                    }
                }
                continue;
            }
            if let Some(target) = choices.as_deref_mut() {
                if self.consume_ident("choices") {
                    match self.parse_string_array() {
                        Some(values) => *target = values,
                        None => return false,
                    }
                    continue;
                }
            }
            return true;
        }
    }

    fn parse_string_array(&mut self) -> Option<Vec<String>> {
        if !self.consume_sym('[') {
            let span = self.span_here();
            self.error(span, "expected `[` to open a string list", None);
            return None;
        }
        let mut values = Vec::new();
        loop {
            if self.consume_sym(']') {
                return Some(values);
            }
            match self.advance().map(|t| t.tok) {
                Some(Tok::Str(value)) => values.push(value),
                Some(Tok::Sym(',')) => {}
                other => {
                    let span = self.span_here();
                    self.error(
                        span,
                        format!("expected a string in list, found {other:?}"),
                        None,
                    );
                    return None;
                }
            }
        }
    }

    fn parse_prompt(&mut self) -> Option<Prompt> {
        match self.advance().map(|t| t.tok) {
            Some(Tok::Str(text)) => Some(Prompt {
                text,
                content_type: None,
            }),
            Some(Tok::TripleStr { text, content_type }) => Some(Prompt { text, content_type }),
            _ => {
                let span = self.span_here();
                self.error(span, "expected a prompt string", None);
                None
            }
        }
    }

    fn parse_tell(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // tell
        let target = self.ident_text("agent target after `tell`")?;
        let mut binding = None;
        let mut requires = Vec::new();
        let mut timeout_seconds = None;
        if !self.parse_effect_modifiers(&mut binding, &mut requires, &mut timeout_seconds, None) {
            return None;
        }
        let prompt = self.parse_prompt()?;
        if !self.parse_effect_modifiers(&mut binding, &mut requires, &mut timeout_seconds, None) {
            return None;
        }
        Some(BodyStmt::Effect(EffectStmt {
            kind: BodyEffectKind::Tell { target },
            binding,
            requires,
            timeout_seconds,
            prompt: Some(prompt),
            span: self.span_from(start),
        }))
    }

    fn parse_coerce_call(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // coerce
        let name = self.ident_text("coerce function name")?;
        if !self.consume_sym('(') {
            let span = self.span_here();
            self.error(span, "expected `(` after coerce function name", None);
            return None;
        }
        let mut args = Vec::new();
        loop {
            if self.consume_sym(')') {
                break;
            }
            if self.peek().is_none() {
                let span = self.span_here();
                self.error(span, "unclosed coerce argument list", None);
                return None;
            }
            let Some((source, _)) = self.parse_value_expression() else {
                return None;
            };
            args.push(source);
            self.consume_sym(',');
        }
        let mut binding = None;
        let mut requires = Vec::new();
        let mut timeout_seconds = None;
        if !self.parse_effect_modifiers(&mut binding, &mut requires, &mut timeout_seconds, None) {
            return None;
        }
        Some(BodyStmt::Effect(EffectStmt {
            kind: BodyEffectKind::Coerce { name, args },
            binding,
            requires,
            timeout_seconds,
            prompt: None,
            span: self.span_from(start),
        }))
    }

    fn parse_ask_human(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // askHuman
        let mut binding = None;
        let mut requires = Vec::new();
        let mut timeout_seconds = None;
        let mut choices = Vec::new();
        if !self.parse_effect_modifiers(
            &mut binding,
            &mut requires,
            &mut timeout_seconds,
            Some(&mut choices),
        ) {
            return None;
        }
        let prompt = self.parse_prompt()?;
        if !self.parse_effect_modifiers(
            &mut binding,
            &mut requires,
            &mut timeout_seconds,
            Some(&mut choices),
        ) {
            return None;
        }
        Some(BodyStmt::Effect(EffectStmt {
            kind: BodyEffectKind::AskHuman { choices },
            binding,
            requires,
            timeout_seconds,
            prompt: Some(prompt),
            span: self.span_from(start),
        }))
    }

    fn parse_decide(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // decide
        let prompt = self.parse_prompt()?;
        if !matches!(self.advance().map(|t| t.tok), Some(Tok::Arrow)) {
            let span = self.span_here();
            self.error(
                span,
                "expected `->` after the decide prompt".to_owned(),
                Some("write `decide \"...\" -> { field type, ... } as name`".to_owned()),
            );
            return None;
        }
        if !self.consume_sym('{') {
            let span = self.span_here();
            self.error(span, "expected `{` to open the decide result shape", None);
            return None;
        }
        let mut result_fields = Vec::new();
        loop {
            if self.consume_sym('}') {
                break;
            }
            let name = self.ident_text("result field name")?;
            let ty = self.ident_text("result field type")?;
            result_fields.push((name, ty));
            self.consume_sym(',');
        }
        let mut binding = None;
        let mut requires = Vec::new();
        let mut timeout_seconds = None;
        if !self.parse_effect_modifiers(&mut binding, &mut requires, &mut timeout_seconds, None) {
            return None;
        }
        if binding.is_none() {
            let span = self.span_from(start);
            self.error(
                span,
                "`decide` requires an `as` binding".to_owned(),
                Some(
                    "the typed result is only reachable through `after <binding> succeeds`"
                        .to_owned(),
                ),
            );
        }
        Some(BodyStmt::Effect(EffectStmt {
            kind: BodyEffectKind::Decide { result_fields },
            binding,
            requires,
            timeout_seconds,
            prompt: Some(prompt),
            span: self.span_from(start),
        }))
    }

    fn parse_call(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // call
        let capability = self.ident_text("plugin capability after `call`")?;
        let argument = if self.consume_ident("for") {
            Some(self.ident_text("argument binding after `for`")?)
        } else {
            None
        };
        let mut binding = None;
        let mut requires = Vec::new();
        let mut timeout_seconds = None;
        if !self.parse_effect_modifiers(&mut binding, &mut requires, &mut timeout_seconds, None) {
            return None;
        }
        Some(BodyStmt::Effect(EffectStmt {
            kind: BodyEffectKind::Call {
                capability,
                argument,
            },
            binding,
            requires,
            timeout_seconds,
            prompt: None,
            span: self.span_from(start),
        }))
    }

    fn parse_invoke(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // invoke
        let workflow = self.ident_text("workflow name after `invoke`")?;
        let payload = self.parse_field_block(false)?;
        let mut binding = None;
        let mut requires = Vec::new();
        let mut timeout_seconds = None;
        if !self.parse_effect_modifiers(&mut binding, &mut requires, &mut timeout_seconds, None) {
            return None;
        }
        Some(BodyStmt::Effect(EffectStmt {
            kind: BodyEffectKind::Invoke { workflow, payload },
            binding,
            requires,
            timeout_seconds,
            prompt: None,
            span: self.span_from(start),
        }))
    }

    fn parse_timer(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // timer
        let span = self.span_here();
        // Absolute deadline: `timer until <time-expr>` (spec/scheduled-time.md).
        if matches!(self.peek().map(|t| &t.tok), Some(Tok::Ident(word)) if word == "until") {
            self.pos += 1; // until
            let until = match self.peek().map(|t| t.tok.clone()) {
                Some(Tok::Str(literal)) => {
                    self.pos += 1;
                    if !is_iso8601_instant(&literal) {
                        self.error(
                            span,
                            format!("invalid time literal `{literal}`"),
                            Some(
                                "use an ISO-8601 instant such as `\"2026-06-15T09:00:00Z\"`"
                                    .to_owned(),
                            ),
                        );
                        return None;
                    }
                    literal
                }
                Some(Tok::Ident(path)) => {
                    // a time-typed path, possibly dotted
                    let mut text = path;
                    self.pos += 1;
                    while matches!(self.peek().map(|t| &t.tok), Some(Tok::Sym('.'))) {
                        self.pos += 1;
                        if let Some(Tok::Ident(seg)) = self.peek().map(|t| t.tok.clone()) {
                            text.push('.');
                            text.push_str(&seg);
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    text
                }
                _ => {
                    self.error(
                        span,
                        "expected a time literal or path after `timer until`".to_owned(),
                        Some("e.g. `timer until \"2026-06-15T09:00:00Z\" as deadline` or `timer until ticket.dueAt as deadline`".to_owned()),
                    );
                    return None;
                }
            };
            let mut binding = None;
            let mut requires = Vec::new();
            let mut timeout_seconds = None;
            if !self.parse_effect_modifiers(&mut binding, &mut requires, &mut timeout_seconds, None)
            {
                return None;
            }
            if binding.is_none() {
                let span = self.span_from(start);
                self.error(
                    span,
                    "`timer` requires an `as` binding".to_owned(),
                    Some("rules react to the timer with `after <binding> succeeds`".to_owned()),
                );
            }
            return Some(BodyStmt::Effect(EffectStmt {
                kind: BodyEffectKind::Timer {
                    duration_seconds: 0,
                    duration_source: String::new(),
                    until: Some(until),
                },
                binding,
                requires,
                timeout_seconds,
                prompt: None,
                span: self.span_from(start),
            }));
        }
        let Some(Tok::Number(value)) = self.peek().map(|t| t.tok.clone()) else {
            self.error(
                span,
                "expected a duration after `timer`".to_owned(),
                Some(
                    "use `<n><unit>` with unit s, m, h, or d, e.g. `timer 24h as deadline`"
                        .to_owned(),
                ),
            );
            return None;
        };
        self.pos += 1;
        let Some(duration_seconds) = parse_short_duration_seconds(&value).filter(|s| *s > 0) else {
            self.error(
                span,
                format!("invalid timer duration `{value}`"),
                Some("use `<n><unit>` with unit s, m, h, or d".to_owned()),
            );
            return None;
        };
        let mut binding = None;
        let mut requires = Vec::new();
        let mut timeout_seconds = None;
        if !self.parse_effect_modifiers(&mut binding, &mut requires, &mut timeout_seconds, None) {
            return None;
        }
        if binding.is_none() {
            let span = self.span_from(start);
            self.error(
                span,
                "`timer` requires an `as` binding".to_owned(),
                Some("rules react to the timer with `after <binding> succeeds`".to_owned()),
            );
        }
        Some(BodyStmt::Effect(EffectStmt {
            kind: BodyEffectKind::Timer {
                duration_seconds,
                duration_source: value,
                until: None,
            },
            binding,
            requires,
            timeout_seconds,
            prompt: None,
            span: self.span_from(start),
        }))
    }

    fn parse_cancel(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // cancel
        let binding = self.ident_text("effect binding after `cancel`")?;
        Some(BodyStmt::Cancel {
            binding,
            span: self.span_from(start),
        })
    }

    /// `acquire <lease> for <key-expr> [until ttl] as <slot>`: one atomic
    /// attempt with branchable `held`/`contended` outcomes
    /// (spec/coordination.md).
    fn parse_lease_acquire(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // acquire
        let resource = self.ident_text("lease name after `acquire`")?;
        if !self.consume_ident("for") {
            let span = self.span_here();
            self.error(
                span,
                "expected `for <key>` after the lease name".to_owned(),
                Some("write `acquire deploy_slot for r.env as slot`".to_owned()),
            );
            return None;
        }
        let key_expr = self.dotted_path_text("lease key expression")?;
        let mut until_ttl = false;
        if self.at_ident("until") {
            self.pos += 1;
            if !self.consume_ident("ttl") {
                let span = self.span_here();
                self.error(
                    span,
                    "expected `ttl` after `until`".to_owned(),
                    Some("`acquire ... until ttl` is the fire-and-forget form".to_owned()),
                );
                return None;
            }
            until_ttl = true;
        }
        let mut binding = None;
        let mut requires = Vec::new();
        let mut timeout_seconds = None;
        if !self.parse_effect_modifiers(&mut binding, &mut requires, &mut timeout_seconds, None) {
            return None;
        }
        if binding.is_none() {
            let span = self.span_from(start);
            self.error(
                span,
                "`acquire` requires an `as` binding".to_owned(),
                Some(
                    "branch on it with `after <binding> held` and `after <binding> contended`"
                        .to_owned(),
                ),
            );
        }
        Some(BodyStmt::Effect(EffectStmt {
            kind: BodyEffectKind::LeaseAcquire {
                resource,
                key_expr,
                until_ttl,
            },
            binding,
            requires,
            timeout_seconds,
            prompt: None,
            span: self.span_from(start),
        }))
    }

    /// `append <Schema> { fields } to <ledger> [as x]` (spec/coordination.md).
    fn parse_ledger_append(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // append
        let schema = self.ident_text("entry schema after `append`")?;
        let fields = self.parse_field_block(false)?;
        if !self.consume_ident("to") {
            let span = self.span_here();
            self.error(
                span,
                "expected `to <ledger>` after the entry payload".to_owned(),
                Some("write `append Decision { ... } to decisions`".to_owned()),
            );
            return None;
        }
        let ledger = self.ident_text("ledger name after `to`")?;
        let mut binding = None;
        let mut requires = Vec::new();
        let mut timeout_seconds = None;
        if !self.parse_effect_modifiers(&mut binding, &mut requires, &mut timeout_seconds, None) {
            return None;
        }
        Some(BodyStmt::Effect(EffectStmt {
            kind: BodyEffectKind::LedgerAppend {
                ledger,
                schema,
                fields,
            },
            binding,
            requires,
            timeout_seconds,
            prompt: None,
            span: self.span_from(start),
        }))
    }

    fn looks_like_counter_consume(&self) -> bool {
        matches!(self.peek_at(1).map(|t| &t.tok), Some(Tok::Ident(_)))
            && matches!(self.peek_at(2).map(|t| &t.tok), Some(Tok::Ident(word)) if word == "for")
    }

    /// `consume <counter> for <key-expr> amount <expr> as <binding>`: one
    /// atomic consume with branchable `ok`/`over` outcomes
    /// (spec/coordination.md).
    fn parse_counter_consume(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // consume
        let counter = self.ident_text("counter name after `consume`")?;
        if !self.consume_ident("for") {
            let span = self.span_here();
            self.error(
                span,
                "expected `for <key>` after the counter name".to_owned(),
                Some(
                    "write `consume model_budget for t.customer amount t.estTokens as spend`"
                        .to_owned(),
                ),
            );
            return None;
        }
        let key_expr = self.dotted_path_text("counter key expression")?;
        if !self.consume_ident("amount") {
            let span = self.span_here();
            self.error(
                span,
                "expected `amount <expr>` after the counter key".to_owned(),
                Some(
                    "write `consume model_budget for t.customer amount t.estTokens as spend`"
                        .to_owned(),
                ),
            );
            return None;
        }
        let amount_expr = match self.peek().map(|t| t.tok.clone()) {
            Some(Tok::Number(value)) => {
                self.pos += 1;
                value
            }
            Some(Tok::Ident(_)) => self.dotted_path_text("consume amount")?,
            _ => {
                let span = self.span_here();
                self.error(
                    span,
                    "expected a number or path after `amount`".to_owned(),
                    None,
                );
                return None;
            }
        };
        let mut binding = None;
        let mut requires = Vec::new();
        let mut timeout_seconds = None;
        if !self.parse_effect_modifiers(&mut binding, &mut requires, &mut timeout_seconds, None) {
            return None;
        }
        if binding.is_none() {
            let span = self.span_from(start);
            self.error(
                span,
                "`consume` requires an `as` binding".to_owned(),
                Some(
                    "branch on it with `after <binding> ok` and `after <binding> over`".to_owned(),
                ),
            );
        }
        Some(BodyStmt::Effect(EffectStmt {
            kind: BodyEffectKind::CounterConsume {
                counter,
                key_expr,
                amount_expr,
            },
            binding,
            requires,
            timeout_seconds,
            prompt: None,
            span: self.span_from(start),
        }))
    }

    /// `notify <instance-expr> event <dotted.name> { payload }`.
    fn parse_notify(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // notify
        let target_expr = self.dotted_path_text("target instance after `notify`")?;
        if !self.consume_ident("event") {
            let span = self.span_here();
            self.error(
                span,
                "expected `event <name>` after the notify target".to_owned(),
                Some("write `notify s.target event deploy.finished { ... }`".to_owned()),
            );
            return None;
        }
        let event = self.dotted_path_text("event name after `event`")?;
        let fields = self.parse_field_block(false)?;
        let mut binding = None;
        let mut requires = Vec::new();
        let mut timeout_seconds = None;
        if !self.parse_effect_modifiers(&mut binding, &mut requires, &mut timeout_seconds, None) {
            return None;
        }
        Some(BodyStmt::Effect(EffectStmt {
            kind: BodyEffectKind::Notify {
                target_expr,
                event,
                fields,
            },
            binding,
            requires,
            timeout_seconds,
            prompt: None,
            span: self.span_from(start),
        }))
    }

    /// A possibly-dotted identifier path, returned as source text.
    fn dotted_path_text(&mut self, label: &str) -> Option<String> {
        let mut text = self.ident_text(label)?;
        while matches!(self.peek().map(|t| &t.tok), Some(Tok::Sym('.'))) {
            self.pos += 1;
            let Some(Tok::Ident(segment)) = self.peek().map(|t| t.tok.clone()) else {
                break;
            };
            text.push('.');
            text.push_str(&segment);
            self.pos += 1;
        }
        Some(text)
    }

    fn parse_exec(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // exec
        let target = match self.advance().map(|t| t.tok) {
            Some(Tok::Str(value)) => ExecTarget::RawCommand(value),
            Some(Tok::Ident(name)) => {
                if !self.at_ident("with") {
                    let span = self.span_here();
                    self.error(
                        span,
                        "expected `with <binding>` after exec capability name".to_owned(),
                        Some(format!(
                            "write `exec {name} with input -> Report as result`"
                        )),
                    );
                    return None;
                }
                self.pos += 1; // with
                let Some(Tok::Ident(stdin_binding)) = self.peek().map(|t| t.tok.clone()) else {
                    let span = self.span_here();
                    self.error(
                        span,
                        "expected a record binding after `with`".to_owned(),
                        Some(format!(
                            "write `exec {name} with input -> Report as result`"
                        )),
                    );
                    return None;
                };
                self.pos += 1;
                ExecTarget::Capability {
                    name,
                    stdin_binding,
                }
            }
            _ => {
                let span = self.span_here();
                self.error(
                    span,
                    "expected a command string or capability name after `exec`".to_owned(),
                    Some(
                        "write `exec \"scripts/run-tests.sh\" as tests` or `exec backup_repo with input -> Report as result`"
                            .to_owned(),
                    ),
                );
                return None;
            }
        };
        // `-> Schema` / `-> each Schema`: typed stdout ingestion
        // (spec/json-ingestion.md).
        let mut parse_target = None;
        if matches!(self.peek().map(|t| &t.tok), Some(Tok::Arrow)) {
            self.pos += 1; // ->
            let each = if self.at_ident("each") {
                self.pos += 1;
                true
            } else {
                false
            };
            let Some(Tok::Ident(schema)) = self.peek().map(|t| t.tok.clone()) else {
                let span = self.span_here();
                self.error(
                    span,
                    "expected a schema name after `->`".to_owned(),
                    Some(
                        "write `exec \"report.sh\" -> Report as x` or `exec \"list.sh\" -> each WorkItem`"
                            .to_owned(),
                    ),
                );
                return None;
            };
            self.pos += 1;
            parse_target = Some(ExecParse { schema, each });
        }
        let mut binding = None;
        let mut requires = Vec::new();
        let mut timeout_seconds = None;
        if !self.parse_effect_modifiers(&mut binding, &mut requires, &mut timeout_seconds, None) {
            return None;
        }
        match &parse_target {
            Some(parse) if parse.each && binding.is_some() => {
                let span = self.span_from(start);
                self.error(
                    span,
                    "`-> each` produces a stream of facts, not a single binding".to_owned(),
                    Some("drop the `as` binding and react with `when <Schema> as item`".to_owned()),
                );
            }
            Some(parse) if !parse.each && binding.is_none() => {
                let span = self.span_from(start);
                self.error(
                    span,
                    "`->` without `each` parses one value and needs an `as` binding".to_owned(),
                    Some("write `exec \"report.sh\" -> Report as x` and read it with `after x succeeds as r`".to_owned()),
                );
            }
            _ => {}
        }
        Some(BodyStmt::Effect(EffectStmt {
            kind: BodyEffectKind::Exec {
                target,
                parse_target,
            },
            binding,
            requires,
            timeout_seconds,
            prompt: None,
            span: self.span_from(start),
        }))
    }

    // -- queue verbs ---------------------------------------------------------

    fn parse_queue_file(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // file
        if !self.consume_ident("item") {
            let span = self.span_here();
            self.error(
                span,
                "expected `item` after `file`",
                Some("write `file item into <queue> { ... }`".to_owned()),
            );
            return None;
        }
        if !self.consume_ident("into") {
            let span = self.span_here();
            self.error(span, "expected `into <queue>` after `file item`", None);
            return None;
        }
        let queue = self.ident_text("queue name")?;
        let fields = self.parse_field_block(false)?;
        let mut binding = None;
        let mut requires = Vec::new();
        let mut timeout_seconds = None;
        if !self.parse_effect_modifiers(&mut binding, &mut requires, &mut timeout_seconds, None) {
            return None;
        }
        Some(BodyStmt::Effect(EffectStmt {
            kind: BodyEffectKind::QueueFile { queue, fields },
            binding,
            requires,
            timeout_seconds,
            prompt: None,
            span: self.span_from(start),
        }))
    }

    fn parse_queue_claim(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // claim
        let item = self.ident_text("item binding after `claim`")?;
        if self.at_ident("with") {
            let span = self.span_here();
            self.error(
                span,
                "`claim <item> with <plugin>` was replaced by the queue interface".to_owned(),
                Some("declare a `queue` and write `claim <item> [as x]`".to_owned()),
            );
            self.pos += 1;
            let _ = self.advance();
        }
        let legacy_plugin: Option<String> = None;
        let mut binding = None;
        let mut requires = Vec::new();
        let mut timeout_seconds = None;
        if !self.parse_effect_modifiers(&mut binding, &mut requires, &mut timeout_seconds, None) {
            return None;
        }
        Some(BodyStmt::Effect(EffectStmt {
            kind: BodyEffectKind::QueueClaim {
                item,
                legacy_plugin,
            },
            binding,
            requires,
            timeout_seconds,
            prompt: None,
            span: self.span_from(start),
        }))
    }

    fn parse_queue_release(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // release
        let item = self.ident_text("item binding after `release`")?;
        Some(BodyStmt::Effect(EffectStmt {
            kind: BodyEffectKind::QueueRelease { item },
            binding: None,
            requires: Vec::new(),
            timeout_seconds: None,
            prompt: None,
            span: self.span_from(start),
        }))
    }

    fn parse_queue_finish(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // finish
        let item = self.ident_text("item binding after `finish`")?;
        let fields = if self.at_sym('{') {
            self.parse_field_block(false)?
        } else {
            Vec::new()
        };
        Some(BodyStmt::Effect(EffectStmt {
            kind: BodyEffectKind::QueueFinish { item, fields },
            binding: None,
            requires: Vec::new(),
            timeout_seconds: None,
            prompt: None,
            span: self.span_from(start),
        }))
    }

    // -- blocks --------------------------------------------------------------

    fn parse_after(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // after
        let binding = self.ident_text("effect binding after `after`")?;
        let predicate = match self.advance().map(|t| t.tok) {
            Some(Tok::Ident(word)) => match word.as_str() {
                "succeeds" => AfterPredicate::Succeeds,
                "fails" => AfterPredicate::Fails,
                "completes" => AfterPredicate::Completes,
                "held" => AfterPredicate::Held,
                "contended" => AfterPredicate::Contended,
                "ok" => AfterPredicate::Ok,
                "over" => AfterPredicate::Over,
                other => {
                    let span = self.span_from(start);
                    self.error(
                        span,
                        format!("unsupported `after` predicate `{other}`"),
                        Some(
                            "use `succeeds`, `fails`, `completes`, or a coordination outcome (`held`, `contended`, `ok`, `over`)"
                                .to_owned(),
                        ),
                    );
                    return None;
                }
            },
            _ => {
                let span = self.span_here();
                self.error(span, "expected `succeeds`, `fails`, or `completes`", None);
                return None;
            }
        };
        let alias = if self.consume_ident("as") {
            Some(self.ident_text("alias after `as`")?)
        } else {
            None
        };
        if !self.consume_sym('{') {
            let span = self.span_here();
            self.error(span, "expected `{` to open the `after` block", None);
            return None;
        }
        let body = self.parse_statements(true);
        Some(BodyStmt::After(AfterBlock {
            binding,
            predicate,
            alias,
            body,
            span: self.span_from(start),
        }))
    }

    fn parse_case(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // case
        let scrutinee = self.ident_text("case scrutinee path")?;
        if !self.consume_sym('{') {
            let span = self.span_here();
            self.error(span, "expected `{` to open the `case` block", None);
            return None;
        }
        let mut branches = Vec::new();
        loop {
            if self.consume_sym('}') {
                break;
            }
            if self.peek().is_none() {
                let span = self.span_here();
                self.error(span, "unclosed `case` block", Some("add `}`".to_owned()));
                break;
            }
            let branch_start = self.pos;
            let pattern = match self.advance().map(|t| t.tok) {
                Some(Tok::Ident(value)) => value,
                Some(Tok::Str(value)) => format!("{value:?}"),
                _ => {
                    let span = self.span_here();
                    self.error(span, "expected a case pattern", None);
                    self.recover();
                    continue;
                }
            };
            let binding = match self.peek().map(|t| t.tok.clone()) {
                // `Variant as binding` (sum types, spec/sum-types.md) — `as`
                // is how every other binding in the language is introduced.
                Some(Tok::Ident(value)) if value == "as" => {
                    self.pos += 1;
                    match self.peek().map(|t| t.tok.clone()) {
                        Some(Tok::Ident(name)) => {
                            self.pos += 1;
                            Some(name)
                        }
                        _ => {
                            let span = self.span_here();
                            self.error(
                                span,
                                "expected a binding name after `as`".to_owned(),
                                Some("write `Variant as payload => { ... }`".to_owned()),
                            );
                            None
                        }
                    }
                }
                Some(Tok::Ident(value)) if value != "where" => {
                    self.pos += 1;
                    Some(value)
                }
                _ => None,
            };
            let guard = if self.consume_ident("where") {
                let guard_start = self.pos;
                // Consume guard tokens up to `=>`.
                while self.peek().is_some()
                    && !matches!(self.peek().map(|t| &t.tok), Some(Tok::FatArrow))
                {
                    self.pos += 1;
                }
                let first = self.tokens.get(guard_start);
                let last = self.tokens.get(self.pos.saturating_sub(1));
                match (first, last) {
                    (Some(first), Some(last)) if guard_start < self.pos => {
                        Some(self.source[first.start..last.end].to_owned())
                    }
                    _ => None,
                }
            } else {
                None
            };
            if !matches!(self.advance().map(|t| t.tok), Some(Tok::FatArrow)) {
                let span = self.span_here();
                self.error(span, "expected `=>` after case pattern", None);
                self.recover();
                continue;
            }
            if !self.consume_sym('{') {
                let span = self.span_here();
                self.error(span, "expected `{` to open the case branch", None);
                self.recover();
                continue;
            }
            let body = self.parse_statements(true);
            branches.push(CaseBranch {
                pattern,
                binding,
                guard,
                body,
                span: self.span_from(branch_start),
            });
        }
        Some(BodyStmt::Case(CaseBlock {
            scrutinee,
            branches,
            span: self.span_from(start),
        }))
    }

    fn parse_branch(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // when
        let Some((condition_source, condition)) = self.parse_value_expression() else {
            return None;
        };
        if !self.consume_sym('{') {
            let span = self.span_here();
            self.error(span, "expected `{` to open the branch body", None);
            return None;
        }
        let then_body = self.parse_statements(true);
        let else_body = if self.consume_ident("else") {
            if !self.consume_sym('{') {
                let span = self.span_here();
                self.error(span, "expected `{` after `else`", None);
                return None;
            }
            Some(self.parse_statements(true))
        } else {
            None
        };
        Some(BodyStmt::Branch(BranchBlock {
            condition_source,
            condition,
            then_body,
            else_body,
            span: self.span_from(start),
        }))
    }

    fn parse_handler(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        self.pos += 1; // on
        let kind = match self.advance().map(|t| t.tok) {
            Some(Tok::Ident(word)) => match word.as_str() {
                "fails" => HandlerKind::OnFails,
                "timeout" => HandlerKind::OnTimeout,
                other => {
                    let span = self.span_from(start);
                    self.error(
                        span,
                        format!("unknown handler `on {other}`"),
                        Some("use `on fails` or `on timeout`".to_owned()),
                    );
                    return None;
                }
            },
            _ => {
                let span = self.span_here();
                self.error(span, "expected `fails` or `timeout` after `on`", None);
                return None;
            }
        };
        if !self.consume_sym('{') {
            let span = self.span_here();
            self.error(span, "expected `{` to open the handler body", None);
            return None;
        }
        let body = self.parse_statements(true);
        Some(BodyStmt::Handler(HandlerBlock {
            kind,
            body,
            span: self.span_from(start),
        }))
    }

    fn parse_terminal(&mut self) -> Option<BodyStmt> {
        let start = self.pos;
        let keyword = match self.advance()?.tok {
            Tok::Ident(value) => value,
            _ => return None,
        };
        let kind = if keyword == "complete" {
            TerminalKind::Complete
        } else {
            TerminalKind::Fail
        };
        let name = self.ident_text("terminal contract name")?;
        let fields = self.parse_field_block(false)?;
        Some(BodyStmt::Terminal(TerminalStmt {
            kind,
            name,
            fields,
            span: self.span_from(start),
        }))
    }
}

const STATEMENT_KEYWORDS: &[&str] = &[
    "record", "done", "consume", "tell", "coerce", "askHuman", "claim", "release", "finish",
    "file", "call", "invoke", "after", "case", "complete", "fail", "timer", "cancel", "decide",
    "exec", "when", "on", "else",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(source: &str) -> BodyAst {
        let (ast, diagnostics) = parse_rule_body(source, 0, BodyMode::Rule);
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
        ast
    }

    #[test]
    fn parses_single_line_record_fields() {
        let ast = parse_ok(r#"record Item { id "a" status "done" }"#);
        let BodyStmt::Record(record) = &ast.statements[0] else {
            panic!("expected record");
        };
        assert_eq!(record.schema, "Item");
        assert_eq!(record.fields.len(), 2);
        assert_eq!(record.fields[0].name, "id");
        assert_eq!(record.fields[1].name, "status");
    }

    #[test]
    fn parses_multi_line_record_with_expressions() {
        let ast = parse_ok(
            "record Job {\n  id job.id\n  attempts job.attempts + 1\n  status \"pending\"\n}",
        );
        let BodyStmt::Record(record) = &ast.statements[0] else {
            panic!("expected record");
        };
        assert_eq!(record.fields[1].name, "attempts");
        let FieldValue::Expr { source, .. } = &record.fields[1].value else {
            panic!("expected expression value");
        };
        assert_eq!(source, "job.attempts + 1");
    }

    #[test]
    fn parses_done_with_replacement() {
        let ast = parse_ok("done task -> record Done {\n  id task.id\n}");
        let BodyStmt::Done {
            binding,
            replacement,
            deprecated_consume,
            ..
        } = &ast.statements[0]
        else {
            panic!("expected done");
        };
        assert_eq!(binding, "task");
        assert!(replacement.is_some());
        assert!(!deprecated_consume);
    }

    #[test]
    fn flags_consume_as_deprecated() {
        let ast = parse_ok("consume task");
        let BodyStmt::Done {
            deprecated_consume, ..
        } = &ast.statements[0]
        else {
            panic!("expected done");
        };
        assert!(deprecated_consume);
    }

    #[test]
    fn parses_tell_with_modifiers_and_prompt() {
        let ast = parse_ok(
            "tell worker requires [\"agent.tell\"] as turn timeout 10m \"\"\"markdown\nDo it.\n\"\"\"",
        );
        let BodyStmt::Effect(effect) = &ast.statements[0] else {
            panic!("expected effect");
        };
        assert_eq!(effect.binding.as_deref(), Some("turn"));
        assert_eq!(effect.requires, vec!["agent.tell".to_owned()]);
        assert_eq!(effect.timeout_seconds, Some(600));
        let prompt = effect.prompt.as_ref().expect("prompt");
        assert_eq!(prompt.content_type.as_deref(), Some("markdown"));
        assert_eq!(prompt.text, "Do it.");
    }

    #[test]
    fn rejects_unknown_statement() {
        let (_, diagnostics) = parse_rule_body("frobnicate task", 0, BodyMode::Rule);
        assert!(diagnostics.iter().any(|d| d
            .message
            .contains("unknown rule body statement `frobnicate`")));
    }

    #[test]
    fn rejects_emit_with_removal_message() {
        let (_, diagnostics) = parse_rule_body("emit event.name", 0, BodyMode::Rule);
        assert!(diagnostics
            .iter()
            .any(|d| d.message.contains("`emit` was removed")));
    }

    #[test]
    fn parses_nested_after_blocks() {
        let ast = parse_ok(
            "tell worker as turn \"go\"\n\nafter turn succeeds as done {\n  coerce review(done.summary) as verdict\n\n  after verdict succeeds as v {\n    record Out {\n      ok v.ok\n    }\n  }\n}",
        );
        assert_eq!(ast.statements.len(), 2);
        let BodyStmt::After(after) = &ast.statements[1] else {
            panic!("expected after");
        };
        assert_eq!(after.predicate, AfterPredicate::Succeeds);
        assert_eq!(after.alias.as_deref(), Some("done"));
        assert!(matches!(after.body[1], BodyStmt::After(_)));
    }

    #[test]
    fn parses_timer_and_cancel() {
        let ast =
            parse_ok("timer 24h as deadline\n\nafter deadline succeeds {\n  cancel signoff\n}");
        let BodyStmt::Effect(effect) = &ast.statements[0] else {
            panic!("expected timer effect");
        };
        assert!(matches!(
            effect.kind,
            BodyEffectKind::Timer {
                duration_seconds: 86400,
                ..
            }
        ));
        let BodyStmt::After(after) = &ast.statements[1] else {
            panic!("expected after");
        };
        assert!(matches!(after.body[0], BodyStmt::Cancel { .. }));
    }

    #[test]
    fn parses_decide_with_result_shape() {
        let ast = parse_ok("decide \"Fixed?\" -> { fixed bool, reason string } as verdict");
        let BodyStmt::Effect(effect) = &ast.statements[0] else {
            panic!("expected effect");
        };
        let BodyEffectKind::Decide { result_fields } = &effect.kind else {
            panic!("expected decide");
        };
        assert_eq!(result_fields.len(), 2);
        assert_eq!(effect.binding.as_deref(), Some("verdict"));
    }

    #[test]
    fn parses_queue_verbs() {
        let ast = parse_ok(
            "file item into backlog {\n  title \"Fix login\"\n  body \"Repro...\"\n}\n\nclaim item as lease\nrelease item\nfinish item {\n  summary turn.summary\n}",
        );
        assert_eq!(ast.statements.len(), 4);
        assert!(matches!(
            &ast.statements[0],
            BodyStmt::Effect(EffectStmt { kind: BodyEffectKind::QueueFile { queue, .. }, .. }) if queue == "backlog"
        ));
        assert!(matches!(
            &ast.statements[1],
            BodyStmt::Effect(EffectStmt { kind: BodyEffectKind::QueueClaim { .. }, binding: Some(b), .. }) if b == "lease"
        ));
    }

    #[test]
    fn parses_exec() {
        let ast = parse_ok("exec \"scripts/run-tests.sh\" as tests timeout 5m");
        let BodyStmt::Effect(effect) = &ast.statements[0] else {
            panic!("expected effect");
        };
        assert!(matches!(&effect.kind, BodyEffectKind::Exec {
                target: ExecTarget::RawCommand(command),
                ..
            } if command == "scripts/run-tests.sh"));
        assert_eq!(effect.timeout_seconds, Some(300));
    }

    #[test]
    fn parses_exec_capability() {
        let ast = parse_ok("exec backup_repo with request -> Report as result");
        let BodyStmt::Effect(effect) = &ast.statements[0] else {
            panic!("expected effect");
        };
        assert!(matches!(&effect.kind, BodyEffectKind::Exec {
            target: ExecTarget::Capability { name, stdin_binding },
            parse_target: Some(ExecParse { schema, each: false }),
        } if name == "backup_repo" && stdin_binding == "request" && schema == "Report"));
        assert_eq!(effect.binding.as_deref(), Some("result"));
    }

    #[test]
    fn parses_case_with_branches() {
        let ast = parse_ok(
            "after turn completes {\n  case turn {\n    Completed done => {\n      record Ok {\n        summary done.summary\n      }\n    }\n    Failed failure => {\n      record Bad {\n        reason failure.reason\n      }\n    }\n  }\n}",
        );
        let BodyStmt::After(after) = &ast.statements[0] else {
            panic!("expected after");
        };
        let BodyStmt::Case(case) = &after.body[0] else {
            panic!("expected case");
        };
        assert_eq!(case.branches.len(), 2);
        assert_eq!(case.branches[0].pattern, "Completed");
        assert_eq!(case.branches[0].binding.as_deref(), Some("done"));
    }

    #[test]
    fn flow_mode_parses_branch_and_handler() {
        let (ast, diagnostics) = parse_rule_body(
            "tell worker as turn \"go\"\non fails {\n  fail error {\n    reason \"boom\"\n  }\n}\nwhen turn.summary == \"ok\" {\n  complete result {\n    ok true\n  }\n} else {\n  fail error {\n    reason \"bad\"\n  }\n}",
            0,
            BodyMode::Flow,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(matches!(ast.statements[1], BodyStmt::Handler(_)));
        assert!(matches!(ast.statements[2], BodyStmt::Branch(_)));
    }

    #[test]
    fn rule_mode_rejects_flow_statements() {
        let (_, diagnostics) = parse_rule_body("on fails {\n  cancel x\n}", 0, BodyMode::Rule);
        assert!(diagnostics
            .iter()
            .any(|d| d.message.contains("only valid inside `flow` bodies")));
    }

    #[test]
    fn unknown_effect_modifier_is_rejected_with_span() {
        let (_, diagnostics) =
            parse_rule_body("tell worker as turn frobnicate \"go\"", 0, BodyMode::Rule);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("expected a prompt string")),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn from_block_supports_shorthand_and_overrides() {
        let ast = parse_ok(
            "done task -> record ReviewedPoem from task {\n  provider poet\n  language\n  topic\n  turn poemTurn\n  status \"reviewed\"\n}",
        );
        let BodyStmt::Done {
            replacement: Some(record),
            ..
        } = &ast.statements[0]
        else {
            panic!("expected replacement record");
        };
        assert_eq!(record.from.as_deref(), Some("task"));
        let names: Vec<_> = record.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["provider", "language", "topic", "turn", "status"]
        );
        assert!(matches!(record.fields[1].value, FieldValue::Shorthand));
        assert!(matches!(record.fields[3].value, FieldValue::Expr { .. }));
    }

    #[test]
    fn invoke_with_nested_payload() {
        let ast = parse_ok(
            "invoke ReviewPhase {\n  phase PhaseReviewRequest {\n    id phase.id\n    title phase.title\n  }\n} as review",
        );
        let BodyStmt::Effect(effect) = &ast.statements[0] else {
            panic!("expected effect");
        };
        let BodyEffectKind::Invoke { workflow, payload } = &effect.kind else {
            panic!("expected invoke");
        };
        assert_eq!(workflow, "ReviewPhase");
        assert!(matches!(payload[0].value, FieldValue::Nested { .. }));
    }

    #[test]
    fn single_line_terminal_payload_parses() {
        let ast = parse_ok("complete result { total 2 }");
        let BodyStmt::Terminal(terminal) = &ast.statements[0] else {
            panic!("expected terminal");
        };
        assert_eq!(terminal.fields.len(), 1);
        assert_eq!(terminal.fields[0].name, "total");
    }

    #[test]
    fn spans_are_absolute() {
        let (ast, _) = parse_rule_body("record Item {\n  id \"a\"\n}", 100, BodyMode::Rule);
        let BodyStmt::Record(record) = &ast.statements[0] else {
            panic!("expected record");
        };
        assert_eq!(record.span.start, 100);
        assert!(record.span.end > 100);
    }
}
