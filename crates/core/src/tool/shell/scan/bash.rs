//! Bash, Zsh and POSIX dialects (`scan` in 0.5.1's `shell/scan.ts`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::LazyLock;

use super::{Command, Dialect, MAX_INPUT_LENGTH, MAX_SUBSTITUTION_DEPTH, Opaque, Src, js_space};

type Cmd = Rc<RefCell<Command>>;
type Step<T> = Result<T, Opaque>;

const REDIRECTS: &[&str] = &[
    "&>>", "&>", "<<<", "<<-", "<<", "<>", "<&", ">&", ">|", ">>", ">", "<",
];
const LIST_OPERATORS: &[&str] = &["&&", "||", "|&"];
const ZSH_LIST_OPERATORS: &[&str] = &["&&", "||", "|&", "&!", "&|"];
const CASE_OPERATORS: &[&str] = &[";;&", ";;", ";&", ";|", "&&", "||", "|&"];
const ZSH_CASE_OPERATORS: &[&str] = &[";;&", ";;", ";&", ";|", "&&", "||", "|&", "&!", "&|"];
/// Builtins that bind their operands to variables or evaluate them as variable names.
const BINDING_BUILTINS: &[&str] = &[
    "declare",
    "typeset",
    "local",
    "export",
    "readonly",
    "private",
    "unset",
    "read",
    "getln",
    "vared",
    "getopts",
    "mapfile",
    "readarray",
    "compgen",
    "zparseopts",
    "zregexparse",
    "zformat",
    "zstyle",
    "zstat",
    "stat",
];
const ARITHMETIC_COMPARISONS: &[&str] = &["-eq", "-ne", "-lt", "-le", "-gt", "-ge"];
const ASSIGNMENT_DECLARATIONS: &[&str] = &["declare", "typeset", "export", "readonly", "local"];
const DECLARATIONS: &[&str] = &[
    "declare", "typeset", "export", "readonly", "local", "unset", "unsetenv",
];
const PRECOMMANDS: &[&str] = &["builtin", "command", "time", "coproc", "!"];
const ZSH_PRECOMMANDS: &[&str] = &["noglob", "nocorrect", "exec", "-"];
const ZSH_ARITHMETIC_BUILTINS: &[&str] = &[
    "integer", "float", "exit", "return", "shift", "break", "continue", "bye", "logout",
];
const NON_FUNCTION_KEYWORDS: &[&str] = &[
    "if", "for", "select", "case", "then", "elif", "else", "fi", "do", "done", "in", "esac",
];
/// Characters that can begin a quoting or expansion unit.
const UNIT_STARTS: &str = "\\$'\"`<>";

macro_rules! sticky_re {
    ($name:ident, $pattern:expr) => {
        static $name: LazyLock<fancy_regex::Regex> = LazyLock::new(|| super::sticky($pattern));
    };
}
macro_rules! re {
    ($name:ident, $pattern:expr) => {
        static $name: LazyLock<regex::Regex> = LazyLock::new(|| super::regex($pattern));
    };
}

sticky_re!(
    TOKEN,
    r"[A-Za-z_][A-Za-z0-9_]*(?=(?:\\\n)*(?:[ \t\n;|&()<>]|$))"
);
sticky_re!(BRACE_CLOSE_AHEAD, r"(?:\\\n)*(?:[ \t\n;&|()<>]|$)");
sticky_re!(SPACE_CONTINUATION_AHEAD, r"(?:\\\n)*[ \t\n]");
sticky_re!(NEGATION_AHEAD, r"(?:\\\n)*[ \t\n(]");
const COMPOUND_AHEAD: &str = r"(?=(?:!(?:[ \t]|\\\n)*)*(?:[{(]|\[\[(?:\\\n)*[ \t\n]|(?:if|while|until|for|select|case|repeat)(?![A-Za-z0-9_])))";
static COPROC_AHEAD: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    super::sticky(&format!(
        r"coproc(?:[ \t]|\\\n)+(?:--(?:[ \t]|\\\n)+)?(?:[A-Za-z_][A-Za-z0-9_]*(?:[ \t]|\\\n)+)?{COMPOUND_AHEAD}"
    ))
});
static TIME_AHEAD: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    super::sticky(&format!(
        r"time(?:[ \t]|\\\n)+(?:-p(?:[ \t]|\\\n)+)?(?:--(?:[ \t]|\\\n)+)?{COMPOUND_AHEAD}"
    ))
});
// [function] name [()] then blanks, continuations, and comments before the body.
sticky_re!(
    FUNCTION_HEAD,
    r"(function[ \t](?:[ \t]|\\\n)*)?([A-Za-z_][A-Za-z0-9_.:+@%-]*)?(?:[ \t]|\\\n)*(\([ \t]*\))?((?:[ \t\n]|\\\n|#[^\n]*\n)*)"
);
sticky_re!(
    FUNCTION_BODY,
    r"[{(]|\[\[(?=(?:\\\n)*[ \t\n])|(?:if|while|until|for|select|case|repeat)(?=(?:\\\n)*(?:[ \t\n(]|$))"
);
sticky_re!(
    DO_AHEAD,
    r"(?:[ \t\n;]|\\\n|#[^\n]*(?:\n|$))*(?:do(?=(?:\\\n)*(?:[ \t\n;{(]|$))|\{(?=(?:\\\n)*[ \t\n]))"
);
sticky_re!(NOFORK_OPEN, r"\$\{(?:\\\n)*(?:[ \t\n]|\|)");
// Zsh flags that neither evaluate nor glob a value; s and j take an argument between repeated delimiters.
sticky_re!(
    ZSH_SAFE_FLAGS,
    r"\$\{\((?:[@AaBbCcDEfFikLMmNnOopPqStUuVvWwXZz0-]|q[-+]?|[js]([^({\[<\\])(?:(?!\1)[^)])*\1)*\)"
);
static ZSH_QUOTED_FLAG_ARG: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    fancy_regex::Regex::new(r"[js]([^({\[<\\])(?:(?!\1)[^)])*\1").expect("scanner regex is valid")
});
sticky_re!(ZSH_GLOB_MODIFIER, r"(?:[\^=~#+]|\\\n)*~");
// Trailing Zsh glob qualifiers without code execution (e, +cmd), delimiters (f, u, g, P), modifiers (:), or math expressions.
sticky_re!(
    ZSH_GLOB_QUALIFIER,
    r"\((?:[/F.@=*rwxAIERWsStUG^MTNDn-]|[oO][nLlamcdN]|[amc][Mwhmsd]?[-+]?[0-9]+|L[kKmMpPgGtT]?[-+]?[0-9]+|[lY][-+]?[0-9]+|\[[-+]?[0-9]+(?:,[-+]?[0-9]+)?\]|,)+\)(?=(?:\\\n)*(?:[ \t\n;&|<>)]|$))"
);
sticky_re!(
    ZSH_SUBSCRIPT,
    r"\$(?:[\^=+~#]|\\\n)*(?:[A-Za-z_](?:[A-Za-z0-9_]|\\\n)*|[@*#?$!0-9-])(?:\\\n)*\["
);
sticky_re!(LINE_CONTINUATION, r"(?:\\\n)*");
sticky_re!(
    PARAMETER_PREFIX,
    r"(?:\\\n)*(?:\([^)]*\))?(?:[!#^=+~]|\\\n)*"
);
sticky_re!(
    PARAMETER_NAME,
    r"(?:[A-Za-z_](?:[A-Za-z0-9_]|\\\n)*|[0-9](?:[0-9]|\\\n)*|[@*#?$!-])(?:\\\n)*"
);
sticky_re!(PARAMETER_PROMPT, r"@(?:\\\n)*[EP](?:\\\n)*\}");

re!(EXPANSION, r"\$(?:\\[^\r\u{2028}\u{2029}])*[({]|`");
re!(
    ESCAPE,
    r"\\(?:x[0-9a-fA-F]|[0-7]|u[0-9a-fA-F]|U[0-9a-fA-F])"
);
re!(
    ZSH_SPECIAL_WORD,
    r"^(?:-[A-Za-z]*v)?(?:(?:dis_)?(?:functions|[gs]?aliases)|options|commands)(?:\+?=|\[|$)"
);
re!(
    ZSH_SPECIAL_NAME,
    r"^(?:(?:dis_)?(?:functions|[gs]?aliases)|options|commands)$"
);
re!(
    ZSH_EVAL_OPTION,
    r"^(?:no)?(?:globsubst|kshglob|promptsubst|promptvars|aliases)$"
);
re!(ZSH_OPTION_FLAG, r"^[-+][A-Za-z0-9]*?[oO]");
re!(ARRAY_REPARSE, r"\+?=(?:\(|[$`])");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Close {
    Paren,
    Brace,
    Nofork,
}

/// Shared across one scan: the work budget, the dialect, and whether a bound expansion reaches a
/// variable sink.
struct Ctx {
    remaining: Cell<i64>,
    dialect: Dialect,
    bound_dollar: Cell<bool>,
    bound_open: Cell<bool>,
    bound_bracket: Cell<bool>,
    sink_bracket: Cell<bool>,
    variable_sink: Cell<bool>,
    eval_sink: Cell<bool>,
}

impl Ctx {
    /// Spends one unit of the budget; true when it ran out.
    fn spent(&self) -> bool {
        let remaining = self.remaining.get() - 1;
        self.remaining.set(remaining);
        remaining < 0
    }

    fn bash(&self) -> bool {
        self.dialect == Dialect::Bash
    }
}

/// A word's cooked text, the literal text it decodes to apart from its expansions, and the literal
/// text of its command substitutions' words, which stands in for their output.
#[derive(Debug, Clone, Default)]
struct Text {
    word: String,
    literal: String,
    output: String,
}

impl Text {
    fn append(&mut self, value: &str) {
        self.word.push_str(value);
        self.literal.push_str(value);
    }
}

/// Arithmetic text, a builtin operand that names or binds a variable now, or a value bound for later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Evaluation {
    Arithmetic,
    Binding,
    Deferred,
}

struct Heredoc {
    delimiter: String,
    dash_delimiter: Option<String>,
    quoted: bool,
    tabs: bool,
    end: usize,
    binding: bool,
    owned: bool,
    command: Option<Cmd>,
    start: Option<usize>,
}

type Heredocs = Rc<RefCell<Vec<Heredoc>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    If,
    While,
    Until,
    For,
    Case,
    Repeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Header,
    Condition,
    Pattern,
    Body,
    Do,
}

struct Structure {
    kind: Kind,
    phase: Phase,
    count: u32,
    saw_else: bool,
    saw_in: bool,
    pattern_started: bool,
    parenthesized: bool,
}

struct Scanned {
    commands: Vec<Cmd>,
    end: usize,
}

struct State<'a, 'o> {
    input: &'a Src,
    ctx: &'a Ctx,
    commands: Vec<Cmd>,
    nested: Vec<Cmd>,
    words: Vec<String>,
    raw_words: Vec<String>,
    // Pending heredocs, shared with nested groups: a body starts after the next newline token at any nesting.
    heredocs: Heredocs,
    structures: Vec<Structure>,
    text: Text,
    word_started: bool,
    word_start: usize,
    word_end: usize,
    command_start: Option<usize>,
    redirect_word_count: Option<usize>,
    redirect_prefix_end: Option<usize>,
    // Posix shells such as Dash read `&>` as `&` and `>`, so later words would start another command there.
    ampersand_redirect_words: Option<usize>,
    command_word_index: Option<usize>,
    assignment_word: bool,
    assignment_head_unsafe: bool,
    invalid: Option<Opaque>,
    redirect_target: bool,
    dup_redirect: bool,
    here_string: bool,
    has_redirect: bool,
    has_file_redirect: bool,
    compound_end: bool,
    // A list operator (&&, ||, |, |&) still awaits its right-hand command.
    dangling: bool,
    // Receives each word's literal text when the list is a command substitution, whose output it stands in for.
    outer: Option<&'o mut Text>,
}

impl State<'_, '_> {
    fn structure(&self) -> Option<&Structure> {
        self.structures.last()
    }

    fn statement(&mut self) {
        if let Some(structure) = self.structures.last_mut() {
            structure.count += 1;
        }
    }

    fn in_header(&self) -> bool {
        matches!(
            self.structure().map(|s| s.phase),
            Some(Phase::Header | Phase::Pattern | Phase::Do)
        )
    }

    fn at_command_start(&self) -> bool {
        self.words.is_empty() && !self.has_redirect && !self.compound_end && !self.in_header()
    }

    fn pending(&self) -> bool {
        self.word_started || !self.words.is_empty() || self.has_redirect || self.compound_end
    }

    fn invalid(&mut self, reason: Opaque) {
        self.invalid.get_or_insert(reason);
    }

    /// Ends the current list; false when an operator still awaits its command.
    fn end_list(&mut self) -> bool {
        if self.pending() {
            self.finish_command(false, true);
        }
        !self.dangling
    }

    fn close_list(&mut self, end: usize) -> Step<Scanned> {
        if !self.end_list() {
            return Err(Opaque::InvalidStructure);
        }
        if let Some(reason) = self.invalid {
            return Err(reason);
        }
        if !self.structures.is_empty() {
            return Err(Opaque::CompoundCommand);
        }
        Ok(Scanned {
            commands: std::mem::take(&mut self.commands),
            end,
        })
    }

    fn finish_word(&mut self) {
        if !self.word_started {
            return;
        }
        let ctx = self.ctx;
        if self.text.word.contains("PS4") {
            ctx.eval_sink.set(true);
        }
        let evaluation = self.word_evaluation();
        if !ctx.bash()
            && (self.text.word.contains("NULLCMD")
                || ((self.assignment_word || evaluation == Some(Evaluation::Binding))
                    && ZSH_SPECIAL_WORD.is_match(&self.text.word)))
        {
            self.invalid(Opaque::DynamicExecution);
        }
        if let Some(evaluation) = evaluation
            && evaluates_expansion(&self.text, evaluation, ctx)
        {
            self.invalid(Opaque::DynamicExecution);
        }
        if self.text.word != self.text.literal && self.command_word_index.is_some() {
            let builtin = self.words.get(self.builtin_index()).map(String::as_str);
            if matches!(builtin, Some("declare" | "typeset" | "local")) {
                ctx.eval_sink.set(true);
            }
        }
        if let Some(outer) = self.outer.as_deref_mut() {
            outer.output.push(' ');
            outer.output.push_str(&self.text.literal);
            outer.output.push(' ');
            outer.output.push_str(&self.text.output);
        }
        let redirect_target = self.redirect_target;
        let repeat_header = match self.structures.last_mut() {
            Some(s) if s.kind == Kind::Repeat && s.phase == Phase::Header => {
                if !redirect_target {
                    s.phase = Phase::Do;
                }
                true
            }
            _ => false,
        };
        if !redirect_target && !repeat_header {
            if !self.assignment_word && self.command_word_index.is_none() {
                self.command_word_index = Some(self.words.len());
            }
            self.command_start.get_or_insert(self.word_start);
            self.words.push(self.text.word.clone());
            // Unquoted trailing continuations are ignored syntax, not part of the raw token.
            self.raw_words
                .push(self.input.slice(self.word_start, self.word_end).to_string());
        }
        if redirect_target {
            static DUP_TARGET: LazyLock<regex::Regex> =
                LazyLock::new(|| super::regex(r"^(?:[0-9]+-?|-)$"));
            if !self.dup_redirect
                || !DUP_TARGET.is_match(self.input.slice(self.word_start, self.word_end))
            {
                self.has_file_redirect = true;
            }
            self.redirect_prefix_end.get_or_insert(self.word_end);
        }
        self.redirect_target = false;
        self.dup_redirect = false;
        self.here_string = false;
        self.text = Text::default();
        self.word_started = false;
        self.assignment_word = false;
        self.assignment_head_unsafe = false;
    }

    /// How the shell evaluates the current word's literal text: as arithmetic, or as a value it
    /// binds to a variable that a later $((name)) may evaluate.
    fn word_evaluation(&self) -> Option<Evaluation> {
        if self.redirect_target {
            return self.here_string.then_some(Evaluation::Deferred);
        }
        if let Some(structure) = self.structure() {
            if structure.kind == Kind::Repeat && structure.phase == Phase::Header {
                return Some(Evaluation::Arithmetic);
            }
            if structure.kind == Kind::For && structure.phase == Phase::Header && structure.saw_in {
                return Some(Evaluation::Deferred);
            }
        }
        if self.command_word_index.is_none() {
            return self.assignment_word.then_some(Evaluation::Deferred);
        }
        let name = self.builtin_index();
        let command = self.words.get(name)?.as_str();
        let options = self.words.get(name + 1..).unwrap_or_default();
        let word = self.text.word.as_str();
        if command == "let" || (!self.ctx.bash() && ZSH_ARITHMETIC_BUILTINS.contains(&command)) {
            return Some(Evaluation::Arithmetic);
        }
        if command == "set" {
            static ARRAY: LazyLock<regex::Regex> =
                LazyLock::new(|| super::regex(r"^[-+][A-Za-z0-9]*A"));
            return Some(if options.iter().any(|w| ARRAY.is_match(w)) {
                Evaluation::Binding
            } else {
                Evaluation::Deferred
            });
        }
        if BINDING_BUILTINS.contains(&command) {
            if options
                .iter()
                .any(|w| w.starts_with('-') && w.contains(['E', 'F', 'i', 'n']))
            {
                return Some(Evaluation::Arithmetic);
            }
            // A plain `name=value` declaration without options binds a scalar unless Bash re-parses an existing array.
            if self.assignment_word
                && ASSIGNMENT_DECLARATIONS.contains(&command)
                && options.iter().all(|w| !w.starts_with('-'))
                && (command == "export"
                    || command == "readonly"
                    || self.ctx.dialect == Dialect::Zsh
                    || !ARRAY_REPARSE.is_match(word))
            {
                return Some(Evaluation::Deferred);
            }
            return Some(Evaluation::Binding);
        }
        if command == "printf" || command == "print" {
            static PRINT_V: LazyLock<regex::Regex> =
                LazyLock::new(|| super::regex(r"^-[A-Za-z]*v"));
            let flag = |w: &str| {
                if command == "print" {
                    PRINT_V.is_match(w)
                } else {
                    w.starts_with("-v")
                }
            };
            return (options.iter().any(|w| flag(w)) || flag(word)).then_some(Evaluation::Binding);
        }
        if command == "wait" {
            static WAIT_P: LazyLock<regex::Regex> = LazyLock::new(|| super::regex(r"^-[nf]*p"));
            static WAIT_P_ONLY: LazyLock<regex::Regex> =
                LazyLock::new(|| super::regex(r"^-[nf]*p$"));
            let last = self.words.last().map(String::as_str).unwrap_or("");
            return (WAIT_P.is_match(word) || WAIT_P_ONLY.is_match(last))
                .then_some(Evaluation::Binding);
        }
        if command == "test" || command == "[" {
            let last = self.words.last().map(String::as_str);
            if last == Some("-v") {
                return Some(Evaluation::Binding);
            }
            if !self.ctx.bash() && last == Some("-t") {
                return Some(Evaluation::Arithmetic);
            }
        }
        None
    }

    /// Index of the word naming the builtin that runs, skipping precommand wrappers and their options.
    fn builtin_index(&self) -> usize {
        static EXEC_A: LazyLock<regex::Regex> = LazyLock::new(|| super::regex(r"^-[cl]*a$"));
        let mut name = self.command_word_index.unwrap_or(0);
        let word = |i: usize| self.words.get(i).map(String::as_str).unwrap_or("");
        while PRECOMMANDS.contains(&word(name))
            || (!self.ctx.bash() && ZSH_PRECOMMANDS.contains(&word(name)))
        {
            let precommand = word(name);
            name += 1;
            while precommand != "-" && self.words.get(name).is_some_and(|w| w.starts_with('-')) {
                if precommand == "exec" && EXEC_A.is_match(word(name)) {
                    name += 1;
                }
                name += 1;
            }
        }
        name
    }

    fn take_heredoc_ownership(&self, command: &Cmd, newline: bool) {
        for heredoc in self.heredocs.borrow_mut().iter_mut() {
            if heredoc.owned {
                continue;
            }
            heredoc.owned = true;
            if newline {
                heredoc.command = Some(command.clone());
                heredoc.start = self.command_start;
            }
        }
    }

    fn finish_command(&mut self, boundary: bool, newline: bool) {
        self.finish_word();
        if self.redirect_target {
            self.invalid(Opaque::InvalidRedirect);
        }
        self.redirect_target = false;
        if self.compound_end && !self.words.is_empty() {
            self.invalid(Opaque::InvalidStructure);
        }
        let in_header = self.in_header();
        if let Some(name) = self.command_word_index
            && self.words.get(name).is_none_or(String::is_empty)
        {
            self.invalid(Opaque::InvalidStructure);
        }
        match self.command_word_index {
            Some(name) if !in_header => {
                let resource = self
                    .input
                    .slice(self.command_start.unwrap_or(0), self.word_end)
                    .trim_matches([' ', '\t', '\n'])
                    .to_string();
                let declaration = name == 0
                    && !self.has_redirect
                    && DECLARATIONS.contains(&self.raw_words[0].as_str())
                    && resource.starts_with(&self.raw_words[0]);
                let redirect_word_count = self
                    .redirect_word_count
                    .filter(|&count| count < self.words.len())
                    .map(|count| count - name);
                let command = Rc::new(RefCell::new(Command {
                    resource,
                    words: self.words[name..].to_vec(),
                    raw_words: self.raw_words[name..].to_vec(),
                    declaration,
                    redirect_word_count,
                    ..Default::default()
                }));
                self.commands.push(command.clone());
                self.check_builtin();
                if BINDING_BUILTINS.contains(
                    &self
                        .words
                        .get(self.builtin_index())
                        .map_or("", String::as_str),
                ) {
                    for heredoc in self.heredocs.borrow_mut().iter_mut() {
                        heredoc.binding = true;
                    }
                }
                self.take_heredoc_ownership(&command, newline);
            }
            None if !in_header
                && self.command_start.is_some()
                && (self.has_file_redirect || (self.has_redirect && !self.compound_end)) =>
            {
                let command_start = self.command_start.unwrap_or(0);
                let raw_span = self.input.slice(command_start, self.word_end);
                let leading = raw_span
                    .chars()
                    .take_while(|c| matches!(c, ' ' | '\t' | '\n'))
                    .count();
                let resource: String = raw_span
                    .chars()
                    .skip(leading)
                    .collect::<String>()
                    .trim_end_matches([' ', '\t', '\n'])
                    .to_string();
                if !resource.is_empty() {
                    let length = resource.chars().count();
                    let prefix_end = match self.redirect_prefix_end {
                        Some(end) => (end as i64 - command_start as i64 - leading as i64)
                            .min(length as i64)
                            .max(1) as usize,
                        None => length,
                    };
                    let prefix: String = resource.chars().take(prefix_end).collect();
                    let command = Rc::new(RefCell::new(Command {
                        resource,
                        words: vec![prefix.clone()],
                        raw_words: vec![prefix],
                        redirect_prefix_end: Some(prefix_end),
                        ..Default::default()
                    }));
                    self.commands.push(command.clone());
                    self.take_heredoc_ownership(&command, newline);
                }
            }
            _ => {}
        }
        for heredoc in self.heredocs.borrow_mut().iter_mut() {
            heredoc.owned = true;
        }
        if boundary
            && self.words.is_empty()
            && !self.has_redirect
            && !self.compound_end
            && !in_header
        {
            self.invalid(Opaque::InvalidStructure);
        }
        if self
            .ampersand_redirect_words
            .is_some_and(|limit| self.words.len() > limit)
        {
            self.invalid(Opaque::InvalidRedirect);
        }
        self.commands.append(&mut self.nested);
        if !in_header && (!self.words.is_empty() || self.has_redirect) {
            self.statement();
        }
        self.words.clear();
        self.raw_words.clear();
        self.command_word_index = None;
        self.command_start = None;
        self.dup_redirect = false;
        self.has_redirect = false;
        self.has_file_redirect = false;
        self.redirect_word_count = None;
        self.redirect_prefix_end = None;
        self.ampersand_redirect_words = None;
        self.compound_end = false;
        self.dangling = false;
    }

    /// Builtins that turn on evaluation of later text make the whole script opaque.
    fn check_builtin(&mut self) {
        let builtin = self.builtin_index();
        let name = self.words.get(builtin).map_or("", String::as_str);
        let args = self.words.get(builtin + 1..).unwrap_or_default();
        let dialect = self.ctx.dialect;
        if name == "eval" {
            self.ctx.eval_sink.set(true);
        }
        static KEYWORD: LazyLock<regex::Regex> = LazyLock::new(|| super::regex(r"^-[A-Za-z0-9]*k"));
        static SET_OPTION: LazyLock<regex::Regex> =
            LazyLock::new(|| super::regex(r"^[-+][A-Za-z0-9]*o"));
        static MATCH_FLAG: LazyLock<regex::Regex> =
            LazyLock::new(|| super::regex(r"^-[A-Za-z0-9]*m"));
        let mut opaque = false;
        if name == "set" && args.iter().any(|w| KEYWORD.is_match(w) || w == "keyword") {
            opaque = true;
        }
        // Dash expands an alias defined earlier in the same script, which can name any command.
        if name == "alias" && args.iter().any(|w| w.contains('=')) {
            opaque = true;
        }
        // Zsh options such as globsubst, kshglob, and promptsubst enable evaluation on expansions and print -P.
        if dialect != Dialect::Bash
            && (name == "emulate"
                || ((name == "setopt"
                    || name == "unsetopt"
                    || (name == "set" && args.iter().any(|w| SET_OPTION.is_match(w))))
                    && args.iter().any(|w| {
                        MATCH_FLAG.is_match(w) || {
                            let option = ZSH_OPTION_FLAG
                                .replace(w, "")
                                .to_lowercase()
                                .replace(['_', '-'], "");
                            ZSH_EVAL_OPTION.is_match(&option)
                        }
                    })))
        {
            opaque = true;
        }
        // Bash compgen -C and -F execute their argument immediately to generate completions.
        if dialect != Dialect::Zsh
            && name == "compgen"
            && args
                .iter()
                .any(|w| w.starts_with('-') && w.contains(['C', 'F']))
        {
            opaque = true;
        }
        if opaque {
            self.invalid(Opaque::DynamicExecution);
        }
    }

    /// Whether a construct read as one unit holds a newline token that would start pending heredoc bodies.
    fn crosses_heredoc(&self, start: usize, end: usize) -> bool {
        !self.heredocs.borrow().is_empty() && self.input.slice(start, end).contains('\n')
    }
}

pub fn scan(input: &str, dialect: Dialect) -> Result<Vec<Command>, Opaque> {
    if super::utf16_len(input) > MAX_INPUT_LENGTH {
        return Err(Opaque::InvalidStructure);
    }
    let ctx = Ctx {
        remaining: Cell::new((MAX_INPUT_LENGTH * MAX_SUBSTITUTION_DEPTH) as i64),
        dialect,
        bound_dollar: Cell::new(false),
        bound_open: Cell::new(false),
        bound_bracket: Cell::new(false),
        sink_bracket: Cell::new(false),
        variable_sink: Cell::new(false),
        eval_sink: Cell::new(false),
    };
    let src = Src::new(input.to_string());
    let result = scan_bash(&src, 0, 0, &ctx, None, None, None)?;
    if ctx.bound_dollar.get()
        && ctx.bound_open.get()
        && (ctx.eval_sink.get()
            || ((ctx.bound_bracket.get() || ctx.sink_bracket.get()) && ctx.variable_sink.get()))
    {
        return Err(Opaque::DynamicExecution);
    }
    Ok(result
        .commands
        .into_iter()
        .map(|c| Rc::try_unwrap(c).map_or_else(|rc| rc.borrow().clone(), RefCell::into_inner))
        .collect())
}

fn ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn is_identifier(word: &str) -> bool {
    let mut chars = word.chars();
    chars.next().is_some_and(ident_start) && chars.all(ident)
}

/// `set.includes(c)` where an absent character reads as `""`, which every string includes.
fn includes(set: &str, c: Option<char>) -> bool {
    c.is_none_or(|c| set.contains(c))
}

fn scan_bash(
    input: &Src,
    start: usize,
    depth: usize,
    ctx: &Ctx,
    close: Option<Close>,
    // A group shares its parent's heredocs; a substitution starts its own.
    heredocs: Option<Heredocs>,
    outer: Option<&mut Text>,
) -> Step<Scanned> {
    if depth > MAX_SUBSTITUTION_DEPTH || ctx.remaining.get() < 0 {
        return Err(Opaque::InvalidStructure);
    }
    let shared = heredocs.is_some();
    let mut state = State {
        input,
        ctx,
        commands: Vec::new(),
        nested: Vec::new(),
        words: Vec::new(),
        raw_words: Vec::new(),
        heredocs: heredocs.unwrap_or_default(),
        structures: Vec::new(),
        text: Text::default(),
        word_started: false,
        word_start: start,
        word_end: start,
        command_start: None,
        redirect_word_count: None,
        redirect_prefix_end: None,
        ampersand_redirect_words: None,
        command_word_index: None,
        assignment_word: false,
        assignment_head_unsafe: false,
        invalid: None,
        redirect_target: false,
        dup_redirect: false,
        here_string: false,
        has_redirect: false,
        has_file_redirect: false,
        compound_end: false,
        dangling: false,
        outer,
    };
    let state = &mut state;
    let mut index = start;
    'scan: while index < input.len() {
        'next: {
            if ctx.spent() {
                return Err(Opaque::InvalidStructure);
            }
            let char = input.chars[index];
            // Line continuations are removed before tokens are read.
            if char == '\\' && input.is(index + 1, '\n') {
                index += 1;
                break 'next;
            }
            if !state.word_started {
                state.word_start = index;
                if char == ' ' || char == '\t' {
                    break 'next;
                }
                if char == '}'
                    && state.words.is_empty()
                    && !state.redirect_target
                    && matches!(close, Some(Close::Brace | Close::Nofork))
                    && state.structures.is_empty()
                    && (shared || state.heredocs.borrow().is_empty())
                    && (close == Some(Close::Nofork)
                        || input.sticky(&BRACE_CLOSE_AHEAD, index + 1).is_some())
                {
                    return state.close_list(index);
                }
                if char == '}' {
                    return Err(if state.structures.is_empty() {
                        Opaque::InvalidStructure
                    } else {
                        Opaque::CompoundCommand
                    });
                }
                if let Some(step) = scan_command_start(state, index, depth)? {
                    index = step;
                    break 'next;
                }
            }
            if char == '\\' && index + 1 >= input.len() {
                return Err(Opaque::UnterminatedEscape);
            }
            if !state.assignment_word
                && ("'\"\\".contains(char)
                    || (char == '$' && matches!(input.at(index + 1), Some('\'' | '"'))))
            {
                state.assignment_head_unsafe = true;
            }
            let in_case_pattern = state.structure().is_some_and(|s| s.phase == Phase::Pattern);
            // Elsewhere a parenthesized group inside a word is a Zsh glob qualifier, which can run code.
            let array = (char == '=' || char == '(')
                && state.assignment_word
                && state.text.word.chars().count() > 1
                && state.text.word.ends_with('=')
                && !state.text.word.ends_with("==")
                && state.command_word_index.is_none_or(|i| {
                    ASSIGNMENT_DECLARATIONS
                        .contains(&state.raw_words.get(i).map_or("", String::as_str))
                });
            if char == '=' && (!state.word_started || array) && input.is(index + 1, '(') {
                let end = scan_nested(
                    input,
                    index + 2,
                    depth,
                    ctx,
                    &mut state.nested,
                    Close::Paren,
                    None,
                )?;
                state.word_started = true;
                state.text.word.push_str(input.slice(index, end + 1));
                index = end;
                state.word_end = index + 1;
                break 'next;
            }
            if UNIT_STARTS.contains(char) {
                let allow_bracket = !state.assignment_word
                    && state.command_word_index.is_some()
                    && !DECLARATIONS.contains(&state.raw_words.first().map_or("", String::as_str));
                let unit = scan_unit(
                    input,
                    index,
                    depth,
                    ctx,
                    &mut state.nested,
                    Mode::Word,
                    allow_bracket,
                    Some(&mut state.text),
                )?;
                if let Some(unit) = unit {
                    state.word_started = true;
                    index = unit;
                    state.word_end = index + 1;
                    break 'next;
                }
            }
            if char == '['
                && !in_case_pattern
                && !state.assignment_word
                && !state.assignment_head_unsafe
                && state.command_word_index.is_none()
                && is_identifier(&state.text.word)
            {
                let mut subscript = Vec::new();
                let end = scan_subscript(
                    input,
                    index + 1,
                    depth,
                    ctx,
                    &mut subscript,
                    Span::ASSIGNMENT_SUBSCRIPT,
                )?;
                if operator(input, end + 1, &["+=", "="])?.is_some() {
                    // Dash has no arrays and word-splits unquoted expansions inside `name[subscript]=value`.
                    if ctx.dialect == Dialect::Posix
                        && input.slice(index + 1, end).contains(['$', '`'])
                    {
                        return Err(Opaque::InvalidStructure);
                    }
                    state.nested.append(&mut subscript);
                    state.assignment_word = true;
                    state.text.word.push_str(input.slice(index, end + 1));
                    index = end;
                    state.word_end = index + 1;
                    break 'next;
                }
            }
            if char == '('
                && (array
                    || ((in_case_pattern || ctx.bash())
                        && state.text.word.ends_with(['?', '*', '+', '@', '!'])))
            {
                let mode = if array { Group::Array } else { Group::Pattern };
                let end =
                    scan_array_or_pattern(input, index, depth + 1, ctx, &mut state.nested, mode)?;
                if state.crosses_heredoc(index, end) {
                    return Err(Opaque::Heredoc);
                }
                // Zsh ends an array assignment at its closing parenthesis, so text right after it starts a command.
                if array && end + 1 < input.len() && !" \t\n;&|<>)".contains(input.chars[end + 1]) {
                    return Err(Opaque::InvalidStructure);
                }
                state.word_started = true;
                state.text.word.push_str(input.slice(index, end + 1));
                index = end;
                state.word_end = index + 1;
                break 'next;
            }
            if char == '#' && !state.word_started {
                let Some(newline) = input.index_of('\n', index) else {
                    break 'scan;
                };
                index = newline - 1;
                break 'next;
            }
            if "<>&|;\n".contains(char) {
                index = scan_operator_or_separator(state, index, depth)?;
                break 'next;
            }
            if in_case_pattern && char == ')' {
                state.finish_word();
                index = index.wrapping_sub(1);
                break 'next;
            }
            if char == ')' {
                if close != Some(Close::Paren)
                    || !state.structures.is_empty()
                    || (!shared && !state.heredocs.borrow().is_empty())
                {
                    return Err(Opaque::CompoundCommand);
                }
                return state.close_list(index);
            }
            if char == '('
                && ctx.dialect == Dialect::Zsh
                && state.word_started
                && !in_case_pattern
                && let Some(after) = input.sticky(&ZSH_GLOB_QUALIFIER, index)
            {
                let end = after - 1;
                state.text.append(input.slice(index, end + 1));
                index = end;
                state.word_end = index + 1;
                break 'next;
            }
            if char == '(' {
                return Err(if state.word_started {
                    Opaque::DynamicExecution
                } else {
                    Opaque::CompoundCommand
                });
            }
            if js_space(char) && !" \t\n".contains(char) {
                return Err(Opaque::InvalidStructure);
            }
            if char == ' ' || char == '\t' {
                state.finish_word();
                break 'next;
            }
            state.word_started = true;
            if char == '='
                && !state.assignment_word
                && !state.assignment_head_unsafe
                && is_identifier(
                    state
                        .text
                        .word
                        .strip_suffix('+')
                        .unwrap_or(&state.text.word),
                )
            {
                state.assignment_word = true;
            }
            let mut buffer = [0; 4];
            state.text.append(char.encode_utf8(&mut buffer));
            state.word_end = index + 1;
        }
        index = index.wrapping_add(1);
    }

    if let Some(close) = close {
        return Err(if close == Close::Paren {
            Opaque::CommandSubstitution
        } else {
            Opaque::InvalidStructure
        });
    }
    if !state.heredocs.borrow().is_empty() {
        return Err(Opaque::Heredoc);
    }
    state.close_list(input.len())
}

/// Steps return the last consumed index.
fn scan_operator_or_separator(state: &mut State, index: usize, depth: usize) -> Step<usize> {
    let input = state.input;
    let ctx = state.ctx;
    let char = input.chars[index];
    let redirect = if "<>&".contains(char) {
        operator(input, index, REDIRECTS)?
    } else {
        None
    };
    let case_head = state
        .structure()
        .is_some_and(|s| s.kind == Kind::Case && s.phase != Phase::Body);
    if redirect.is_some() && case_head {
        return Err(Opaque::InvalidRedirect);
    }
    if let Some(redirect) = redirect {
        let length = redirect.len();
        state.has_redirect = true;
        state.command_start.get_or_insert(state.word_start);
        if state.redirect_target {
            state.invalid(Opaque::InvalidRedirect);
        }
        static FD_PREFIX: LazyLock<regex::Regex> =
            LazyLock::new(|| super::regex(r"^(?:[0-9]+|\{[A-Za-z_][A-Za-z0-9_]*\})$"));
        let fd_prefix = state.word_started
            && !state.assignment_head_unsafe
            && FD_PREFIX.is_match(&state.text.word);
        if fd_prefix {
            state.text = Text::default();
            state.word_started = false;
        } else {
            state.finish_word();
        }
        if redirect.starts_with('&') && ctx.dialect == Dialect::Posix {
            state
                .ampersand_redirect_words
                .get_or_insert(state.words.len());
        }
        if state.redirect_word_count.is_none() && state.command_word_index.is_some() {
            state.redirect_word_count = Some(state.words.len());
        }
        if redirect == "<<" || redirect == "<<-" {
            let Some(delimiter) = heredoc_delimiter(input, index + length, redirect == "<<-")
            else {
                return Err(Opaque::InvalidRedirect);
            };
            state.has_file_redirect = true;
            let end = delimiter.end;
            state
                .redirect_prefix_end
                .get_or_insert(if includes(" \t", input.at(index + length)) {
                    index + length
                } else {
                    end + 1
                });
            state.heredocs.borrow_mut().push(delimiter);
            state.word_end = end + 1;
            state.redirect_target = false;
            state.dup_redirect = false;
            return Ok(end);
        }
        if includes(" \t\n", input.at(index + length)) {
            state.redirect_prefix_end.get_or_insert(index + length);
        }
        state.redirect_target = true;
        state.dup_redirect = redirect == "<&" || redirect == ">&";
        state.here_string = redirect == "<<<";
        return Ok(index + length - 1);
    }
    if state.structure().is_some_and(|s| s.phase == Phase::Pattern) && char == '|' {
        state.finish_word();
        return Ok(index.wrapping_sub(1));
    }
    let case_body = state
        .structure()
        .is_some_and(|s| s.kind == Kind::Case && s.phase == Phase::Body);
    let operators = match (ctx.dialect == Dialect::Zsh, case_body) {
        (true, true) => ZSH_CASE_OPERATORS,
        (true, false) => ZSH_LIST_OPERATORS,
        (false, true) => CASE_OPERATORS,
        (false, false) => LIST_OPERATORS,
    };
    let found = operator(input, index, operators)?;
    let mut buffer = [0; 4];
    let separator: &str = found.unwrap_or_else(|| char.encode_utf8(&mut buffer));
    if case_body && separator.starts_with(';') && separator.len() > 1 {
        if !state.end_list() {
            return Err(Opaque::InvalidStructure);
        }
        if let Some(structure) = state.structures.last_mut() {
            structure.phase = Phase::Pattern;
            structure.pattern_started = false;
        }
        return Ok(index + separator.len() - 1);
    }
    let (kind, phase, saw_in) = match state.structure() {
        Some(s) => (Some(s.kind), Some(s.phase), s.saw_in),
        None => (None, None, false),
    };
    if kind == Some(Kind::Case) && matches!(phase, Some(Phase::Header | Phase::Pattern)) {
        if separator != "\n" {
            return Err(Opaque::CompoundCommand);
        }
        if phase == Some(Phase::Pattern) && (state.word_started || !state.words.is_empty()) {
            return Err(Opaque::CompoundCommand);
        }
        state.finish_word();
        return read_heredocs(state, index, depth);
    }
    if matches!(kind, Some(Kind::For | Kind::Repeat))
        && matches!(phase, Some(Phase::Header | Phase::Do))
    {
        if separator != ";" && separator != "\n" {
            return Err(Opaque::CompoundCommand);
        }
        if phase == Some(Phase::Header) && !saw_in && !state.word_started && state.words.is_empty()
        {
            return Err(Opaque::CompoundCommand);
        }
    }
    if separator == "\n" && !state.pending() {
        return read_heredocs(state, index, depth);
    }
    if separator == ";" && ctx.dialect == Dialect::Zsh && !state.pending() && !state.dangling {
        return Ok(index);
    }
    let length = separator.len();
    let newline = separator == "\n";
    let dangling =
        !(separator == ";" || newline || (separator.starts_with('&') && separator != "&&"));
    state.finish_command(true, newline);
    if kind == Some(Kind::For)
        && phase == Some(Phase::Header)
        && let Some(structure) = state.structures.last_mut()
    {
        structure.phase = Phase::Do;
    }
    state.dangling = dangling;
    // Reprocess the newline to read pending heredoc bodies.
    if newline && !state.heredocs.borrow().is_empty() {
        return Ok(index.wrapping_sub(1));
    }
    Ok(index + length - 1)
}

fn command_key(command: &Command) -> (String, Vec<String>) {
    (command.resource.clone(), command.words.clone())
}

/// Reads the pending heredoc bodies after the newline at `index` and returns the last index they cover.
fn read_heredocs(state: &mut State, index: usize, depth: usize) -> Step<usize> {
    let input = state.input;
    let ctx = state.ctx;
    let mut end = index;
    let pending: Vec<Heredoc> = state.heredocs.borrow_mut().drain(..).collect();
    for heredoc in pending {
        let Some(body) = heredoc_body(input, end + 1, &heredoc) else {
            return Err(Opaque::Heredoc);
        };
        if let Some(command) = &heredoc.command {
            command.borrow_mut().resource =
                super::js_trim(input.slice(heredoc.start.unwrap_or(0), body.end)).to_string();
        }
        let mut text = if heredoc.quoted {
            Text {
                word: body.source.clone(),
                literal: body.source.clone(),
                output: String::new(),
            }
        } else {
            Text::default()
        };
        if !heredoc.quoted {
            let source = Src::new(body.source.clone());
            scan_span(
                &source,
                0,
                depth,
                ctx,
                &mut state.commands,
                Span::HEREDOC,
                true,
                Some(&mut text),
            )?;
        }
        if heredoc.binding {
            evaluates_expansion(&text, Evaluation::Deferred, ctx);
        }
        end = body.end;
    }
    Ok(end)
}

fn scan_command_start(state: &mut State, index: usize, depth: usize) -> Step<Option<usize>> {
    let input = state.input;
    let ctx = state.ctx;
    let char = input.chars[index];
    let entry_len = state.structures.len();
    let (kind, phase, pattern_started) = match state.structure() {
        Some(s) => (Some(s.kind), Some(s.phase), s.pattern_started),
        None => (None, None, false),
    };
    if phase == Some(Phase::Pattern) && !pattern_started && state.words.is_empty() && char == '(' {
        if let Some(s) = state.structures.last_mut() {
            s.pattern_started = true;
        }
        return Ok(Some(index));
    }
    if phase == Some(Phase::Pattern) && char == '|' {
        if state.words.is_empty() {
            return Err(Opaque::CompoundCommand);
        }
        state.finish_word();
        if let Some(s) = state.structures.last_mut() {
            s.pattern_started = true;
        }
        return Ok(Some(index));
    }
    if phase == Some(Phase::Pattern) && char == ')' {
        if state.words.is_empty() {
            return Err(Opaque::CompoundCommand);
        }
        state.finish_command(false, false);
        if let Some(s) = state.structures.last_mut() {
            s.phase = Phase::Body;
            s.count = 0;
        }
        return Ok(Some(index));
    }
    if kind == Some(Kind::For)
        && phase == Some(Phase::Header)
        && char == '('
        && !input.is(index + 1, '(')
    {
        let end = scan_array_or_pattern(
            input,
            index,
            depth + 1,
            ctx,
            &mut state.nested,
            Group::Array,
        )?;
        if state.crosses_heredoc(index, end) {
            return Err(Opaque::Heredoc);
        }
        state.finish_command(false, false);
        // Zsh permits a sublist or brace group directly after the value list, without do/done.
        if let Some(s) = state.structures.last_mut() {
            s.phase = Phase::Do;
            s.parenthesized = true;
        }
        if input.sticky(&DO_AHEAD, end + 1).is_none() {
            state.structures.pop();
        }
        return Ok(Some(end));
    }
    if kind == Some(Kind::Repeat)
        && phase == Some(Phase::Do)
        && !matches!(char, '\n' | '#' | ';')
        && token_at(input, index) != Some("do".to_string())
    {
        state.structures.pop();
    }
    if let Some(step) = scan_keyword(state, index, depth)? {
        return Ok(Some(step));
    }
    // The structure innermost on entry, as it is now; gone when it was popped.
    let entry_phase = |state: &State| {
        (state.structures.len() == entry_len)
            .then(|| state.structure().map(|s| s.phase))
            .flatten()
    };
    if kind == Some(Kind::For)
        && entry_phase(state) == Some(Phase::Do)
        && !matches!(char, '\n' | '#' | ';')
    {
        if char != '{' || input.sticky(&SPACE_CONTINUATION_AHEAD, index + 1).is_none() {
            return Err(Opaque::CompoundCommand);
        }
        state.structures.pop();
    }
    let at_start = state.at_command_start();
    let function_head = if at_start && char == '(' {
        function_head_length(input, index)
    } else {
        0
    };
    if function_head > 0 {
        return Ok(Some(index + function_head - 1));
    }
    if at_start && char == '!' && input.sticky(&NEGATION_AHEAD, index + 1).is_some() {
        return Ok(Some(index));
    }
    let for_header = kind == Some(Kind::For) && entry_phase(state) == Some(Phase::Header);
    let compound_open = if (at_start || for_header) && "([".contains(char) {
        operator(input, index, &["((", "[["])?
    } else {
        None
    };
    if compound_open == Some("((") {
        if for_header && !state.words.is_empty() {
            return Err(Opaque::CompoundCommand);
        }
        let span = if for_header {
            Span::FOR_ARITHMETIC
        } else {
            Span::ARITHMETIC
        };
        match scan_arithmetic(input, index + 2, depth, ctx, &mut state.commands, span)? {
            Some(end) => {
                if state.crosses_heredoc(index, end) {
                    return Err(Opaque::Heredoc);
                }
                if !for_header && ctx.dialect == Dialect::Posix {
                    // Dash reads `((` as two nested subshells, so posix reports both readings.
                    let subshells = scan_bash(
                        input,
                        index + 1,
                        depth + 1,
                        ctx,
                        Some(Close::Paren),
                        None,
                        None,
                    )?;
                    if subshells.end != end {
                        return Err(Opaque::CompoundCommand);
                    }
                    let seen: std::collections::HashSet<_> = state
                        .commands
                        .iter()
                        .map(|c| command_key(&c.borrow()))
                        .collect();
                    state.commands.extend(
                        subshells
                            .commands
                            .into_iter()
                            .filter(|c| !seen.contains(&command_key(&c.borrow()))),
                    );
                }
                if for_header {
                    if let Some(s) = state.structures.last_mut() {
                        s.phase = Phase::Do;
                        s.saw_in = true;
                    }
                } else {
                    state.statement();
                    state.compound_end = true;
                }
                return Ok(Some(end));
            }
            None if for_header => return Err(Opaque::InvalidStructure),
            None => {}
        }
    }
    if at_start
        && (char == '('
            || (char == '{' && input.sticky(&SPACE_CONTINUATION_AHEAD, index + 1).is_some()))
    {
        let close = if char == '{' {
            Close::Brace
        } else {
            Close::Paren
        };
        let group = scan_bash(
            input,
            index + 1,
            depth + 1,
            ctx,
            Some(close),
            Some(state.heredocs.clone()),
            None,
        )?;
        if super::js_trim(input.slice(index + 1, group.end)).is_empty() {
            return Err(Opaque::InvalidStructure);
        }
        state.commands.extend(group.commands);
        state.statement();
        state.compound_end = true;
        return Ok(Some(group.end));
    }
    if at_start
        && compound_open == Some("[[")
        && input.sticky(&SPACE_CONTINUATION_AHEAD, index + 2).is_some()
    {
        let end = scan_conditional(input, index + 2, depth, ctx, &mut state.commands)?;
        if state.crosses_heredoc(index, end) {
            return Err(Opaque::Heredoc);
        }
        state.statement();
        state.compound_end = true;
        return Ok(Some(end));
    }
    Ok(None)
}

fn token_at(input: &Src, index: usize) -> Option<String> {
    input
        .sticky(&TOKEN, index)
        .map(|end| input.slice(index, end).to_string())
}

fn scan_keyword(state: &mut State, index: usize, depth: usize) -> Step<Option<usize>> {
    let input = state.input;
    let ctx = state.ctx;
    let char = input.chars[index];
    if !ident_start(char) {
        return Ok(None);
    }
    let token = token_at(input, index);
    let token = token.as_deref();
    let end = (index + token.map_or(0, str::len)).wrapping_sub(1);
    let (kind, phase, saw_in, parenthesized, pattern_started) = match state.structure() {
        Some(s) => (
            Some(s.kind),
            Some(s.phase),
            s.saw_in,
            s.parenthesized,
            s.pattern_started,
        ),
        None => (None, None, false, false, false),
    };
    if kind == Some(Kind::Case) && phase == Some(Phase::Header) && token == Some("in") {
        if state.words.len() != 1 || state.has_redirect {
            return Err(Opaque::CompoundCommand);
        }
        state.finish_command(false, false);
        if let Some(s) = state.structures.last_mut() {
            s.phase = Phase::Pattern;
            s.pattern_started = false;
        }
        return Ok(Some(end));
    }
    if kind == Some(Kind::For)
        && matches!(phase, Some(Phase::Header | Phase::Do))
        && !saw_in
        && !parenthesized
        && token == Some("in")
        && (phase == Some(Phase::Do) || !state.words.is_empty())
    {
        if let Some(s) = state.structures.last_mut() {
            s.saw_in = true;
            s.phase = Phase::Header;
        }
        return Ok(Some(end));
    }
    if phase == Some(Phase::Pattern)
        && !pattern_started
        && state.words.is_empty()
        && token == Some("esac")
    {
        state.structures.pop();
        state.statement();
        state.compound_end = true;
        return Ok(Some(end));
    }
    // POSIX `for name do` omits the in list.
    if kind == Some(Kind::For)
        && phase == Some(Phase::Header)
        && !saw_in
        && state.words.len() == 1
        && !state.has_redirect
        && token == Some("do")
    {
        state.finish_command(false, false);
        if let Some(s) = state.structures.last_mut() {
            s.phase = Phase::Do;
        }
    }
    let in_header = state.in_header();
    let phase = state.structure().map(|s| s.phase);
    if let Some(token) = token
        && ["then", "elif", "else", "fi", "do", "done", "esac"].contains(&token)
        && state.words.is_empty()
        && !state.redirect_target
        && (!in_header || (token == "do" && phase == Some(Phase::Do)))
    {
        // Bash and Dash read a reserved word after a redirect as a command word, while Zsh reads a keyword.
        if (state.has_redirect && ctx.dialect != Dialect::Zsh) || !state.end_list() {
            return Err(Opaque::CompoundCommand);
        }
        let Some(structure) = state.structures.last_mut() else {
            return Err(Opaque::CompoundCommand);
        };
        let loop_kind = matches!(
            structure.kind,
            Kind::For | Kind::While | Kind::Until | Kind::Repeat
        );
        match token {
            "then" => {
                if structure.kind != Kind::If
                    || structure.phase != Phase::Condition
                    || structure.count == 0
                {
                    return Err(Opaque::CompoundCommand);
                }
                structure.phase = Phase::Body;
                structure.count = 0;
                return Ok(Some(end));
            }
            "elif" | "else" => {
                if structure.kind != Kind::If
                    || structure.phase != Phase::Body
                    || structure.count == 0
                    || structure.saw_else
                {
                    return Err(Opaque::CompoundCommand);
                }
                structure.phase = if token == "elif" {
                    Phase::Condition
                } else {
                    Phase::Body
                };
                structure.saw_else = token == "else";
                structure.count = 0;
                return Ok(Some(end));
            }
            "do" => {
                if !loop_kind
                    || !matches!(structure.phase, Phase::Condition | Phase::Do)
                    || (structure.kind != Kind::For
                        && structure.kind != Kind::Repeat
                        && structure.count == 0)
                {
                    return Err(Opaque::CompoundCommand);
                }
                structure.phase = Phase::Body;
                structure.count = 0;
                return Ok(Some(end));
            }
            _ => {}
        }
        if (token == "fi"
            && (structure.kind != Kind::If
                || structure.phase != Phase::Body
                || structure.count == 0))
            || (token == "done"
                && (!loop_kind || structure.phase != Phase::Body || structure.count == 0))
            || (token == "esac"
                && (structure.kind != Kind::Case || structure.phase == Phase::Header))
        {
            return Err(Opaque::CompoundCommand);
        }
        state.structures.pop();
        state.statement();
        state.compound_end = true;
        return Ok(Some(end));
    }
    if !state.at_command_start() {
        return Ok(None);
    }
    let definition = function_head_length(input, index);
    if definition > 0 {
        return Ok(Some(index + definition - 1));
    }
    let kind = match token {
        Some("if") => Some(Kind::If),
        Some("while") => Some(Kind::While),
        Some("until") => Some(Kind::Until),
        Some("for" | "select") => Some(Kind::For),
        Some("case") => Some(Kind::Case),
        Some("repeat") if !ctx.bash() => Some(Kind::Repeat),
        _ => None,
    };
    if let Some(kind) = kind {
        if depth + state.structures.len() >= MAX_SUBSTITUTION_DEPTH {
            return Err(Opaque::CompoundCommand);
        }
        state.structures.push(Structure {
            kind,
            phase: if matches!(kind, Kind::For | Kind::Case | Kind::Repeat) {
                Phase::Header
            } else {
                Phase::Condition
            },
            count: 0,
            saw_else: false,
            saw_in: false,
            pattern_started: false,
            parenthesized: false,
        });
        // The compound command itself satisfies a preceding list operator.
        state.dangling = false;
        return Ok(Some(end));
    }
    if token == Some("coproc")
        && let Some(after) = input.sticky(&COPROC_AHEAD, index)
    {
        return Ok(Some(after - 1));
    }
    if token == Some("time")
        && let Some(after) = input.sticky(&TIME_AHEAD, index)
    {
        return Ok(Some(after - 1));
    }
    Ok(None)
}

fn function_head_length(input: &Src, start: usize) -> usize {
    if start > input.len() {
        return 0;
    }
    let rest = input.rest(start);
    let Ok(Some(head)) = FUNCTION_HEAD.captures(rest) else {
        return 0;
    };
    let keyword = head.get(1).is_some();
    let name = head.get(2).map(|m| m.as_str());
    let parens = head.get(3).is_some();
    let rejected = if keyword {
        name.is_none()
    } else {
        !parens || NON_FUNCTION_KEYWORDS.contains(&name.unwrap_or(""))
    };
    if rejected {
        return 0;
    }
    let whole = head.get(0).map_or("", |m| m.as_str()).chars().count();
    let trivia = head.get(4).map_or("", |m| m.as_str()).chars().count();
    // Blanks, comments, and newlines before the body stay for the caller, where newlines start heredoc bodies.
    if input.sticky(&FUNCTION_BODY, start + whole).is_some() {
        whole - trivia
    } else {
        0
    }
}

/// Word text is unquoted. Quoted text follows double-quote rules. Arithmetic text, including
/// subscripts, expands like double-quoted text while its matcher still lets a backslash escape any
/// character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Word,
    Quoted,
    Arithmetic,
}

#[derive(Debug, Clone, Copy)]
struct Span {
    open: Option<char>,
    close: Option<char>,
    reject: &'static str,
    mode: Mode,
}

impl Span {
    const DOUBLE: Span = Span {
        open: None,
        close: Some('"'),
        reject: "",
        mode: Mode::Quoted,
    };
    const HEREDOC: Span = Span {
        open: None,
        close: None,
        reject: "",
        mode: Mode::Quoted,
    };
    // Bash rereads `((...))` it cannot parse as arithmetic, such as text with a comment, as nested subshells.
    const ARITHMETIC: Span = Span {
        open: Some('('),
        close: Some(')'),
        reject: ";#}",
        mode: Mode::Arithmetic,
    };
    const FOR_ARITHMETIC: Span = Span {
        open: Some('('),
        close: Some(')'),
        reject: "#}",
        mode: Mode::Arithmetic,
    };
    const BRACKET_ARITHMETIC: Span = Span {
        open: None,
        close: Some(']'),
        reject: ";|&<>()[\n'\"\\#",
        mode: Mode::Arithmetic,
    };
    const SUBSCRIPT: Span = Span {
        open: Some('['),
        close: Some(']'),
        reject: "\"}",
        mode: Mode::Arithmetic,
    };
    const QUOTED_SUBSCRIPT: Span = Span {
        open: Some('['),
        close: Some(']'),
        reject: "\"",
        mode: Mode::Quoted,
    };
    // Dash splits an assignment subscript at blanks and operators, so Bash and Zsh assignments diverge.
    const ASSIGNMENT_SUBSCRIPT: Span = Span {
        open: Some('['),
        close: Some(']'),
        reject: " \t\n\r\x0B\x0C;&|<>()",
        mode: Mode::Arithmetic,
    };
    // Single quotes are literal here in some shells and quoting in others, so reject what either reading
    // would parse structurally and scan the contents for expansions. Inside a double-quoted parameter, Zsh and
    // Dash read them literally even within subscripts and arithmetic, where `"` and `}` would end the parameter.
    const ARITHMETIC_QUOTE: Span = Span {
        open: None,
        close: Some('\''),
        reject: "()[];\"}",
        mode: Mode::Quoted,
    };
    const ARITHMETIC_DOUBLE: Span = Span {
        open: None,
        close: Some('"'),
        reject: "()[];}",
        mode: Mode::Quoted,
    };
    const PARAMETER_QUOTE: Span = Span {
        open: None,
        close: Some('\''),
        reject: "\"}[]",
        mode: Mode::Quoted,
    };
}

/// Scans one quoting or expansion unit at `index` into `commands`, appending its text to `text`.
/// Returns the unit's last index, or `None` when the character is ordinary text.
#[allow(clippy::too_many_arguments)]
fn scan_unit(
    input: &Src,
    index: usize,
    depth: usize,
    ctx: &Ctx,
    commands: &mut Vec<Cmd>,
    mode: Mode,
    allow_bracket: bool,
    mut text: Option<&mut Text>,
) -> Step<Option<usize>> {
    let char = input.chars[index];
    let next = input.at(index + 1).unwrap_or('\0');
    if char == '\\' {
        if mode == Mode::Quoted && !"$`\"\\\n".contains(next) {
            return Ok(None);
        }
        if let Some(text) = text.as_deref_mut()
            && next != '\n'
        {
            text.append(input.slice(index + 1, index + 2));
        }
        return Ok(Some(index + 1));
    }
    if char == '$' && !ctx.bash() && zsh_evaluates(input, index) {
        return Err(Opaque::DynamicExecution);
    }
    if char == '$'
        && !ctx.bash()
        && matches!(mode, Mode::Word | Mode::Quoted)
        && let Some(bracket) = input.sticky(&ZSH_SUBSCRIPT, index)
    {
        let mut subscript = Text::default();
        let span = if mode == Mode::Quoted {
            Span::QUOTED_SUBSCRIPT
        } else {
            Span::SUBSCRIPT
        };
        let end = scan_span(
            input,
            bracket,
            depth,
            ctx,
            commands,
            span,
            false,
            Some(&mut subscript),
        )?;
        ctx.sink_bracket.set(true);
        if evaluates_expansion(&subscript, Evaluation::Arithmetic, ctx) {
            return Err(Opaque::DynamicExecution);
        }
        if let Some(text) = text.as_deref_mut() {
            text.word.push_str(input.slice(index, bracket));
            text.word.push_str(&subscript.word);
            text.word.push(']');
        }
        return Ok(Some(end));
    }
    let opener = if char == '$' {
        operator(input, index, &["$$", "$((", "$(", "${", "$[", "$'", "$\""])?
    } else if mode == Mode::Word && char == '<' {
        operator(input, index, &["<("])?
    } else if mode == Mode::Word && char == '>' {
        operator(input, index, &[">("])?
    } else {
        None
    };
    if opener == Some("$$") {
        let quote_start = input
            .sticky(&LINE_CONTINUATION, index + 2)
            .unwrap_or(index + 2);
        if input.is(quote_start, '\'') {
            let Some(next_quote) = input.index_of('\'', quote_start + 1) else {
                return Err(Opaque::UnterminatedQuote);
            };
            // Zsh and Bash 3.2 inside ${...} can parse $$'...' with ANSI-C escapes while Bash 5/Dash treat $$ as PID.
            if input.slice(quote_start + 1, next_quote).contains('\\')
                && input.index_of('\'', next_quote + 1).is_some()
            {
                return Err(Opaque::UnterminatedQuote);
            }
        }
        if let Some(text) = text.as_deref_mut() {
            text.word.push_str("$$");
        }
        return Ok(Some(index + 1));
    }
    if mode == Mode::Word && opener == Some("$'") {
        let Some(quote) = ansi_quote(input, index + 1, true) else {
            return Err(Opaque::UnterminatedQuote);
        };
        let first_quote = input.index_of('\'', index + 2);
        // Dash does not support $'...' and closes the single-quoted span at the first `'`, even after `\`.
        if Some(quote.end) != first_quote {
            static DASH_BREAK: LazyLock<regex::Regex> =
                LazyLock::new(|| super::regex(r#"[#"\n]|\\'"#));
            let zsh_pid = !ctx.bash() && index >= 2 && input.is(index - 2, '$');
            let posix = ctx.dialect == Dialect::Posix && {
                let first = first_quote.unwrap_or(usize::MAX);
                let next = first.checked_add(1).and_then(|f| input.index_of('\'', f));
                next.is_some_and(|n| n < quote.end)
                    || DASH_BREAK.is_match(input.rest(first.saturating_add(1)))
            };
            if zsh_pid || posix {
                return Err(Opaque::UnterminatedQuote);
            }
        }
        if let Some(text) = text.as_deref_mut() {
            text.append(&quote.value);
        }
        return Ok(Some(quote.end));
    }
    if mode == Mode::Arithmetic && opener == Some("$'") {
        let quote = ansi_quote(input, index + 1, true);
        let Some(quote) = quote.filter(|q| {
            Some(q.end) == input.index_of('\'', index + 2)
                && !q.value.contains(['(', ')', '[', ']', ';', '"', '}'])
        }) else {
            return Err(Opaque::InvalidStructure);
        };
        if let Some(text) = text.as_deref_mut() {
            text.append(&quote.value);
        }
        return scan_span(
            input,
            index + 2,
            depth,
            ctx,
            commands,
            Span::ARITHMETIC_QUOTE,
            allow_bracket,
            text,
        )
        .map(Some);
    }
    if mode == Mode::Word && char == '\'' {
        let Some(end) = input.index_of('\'', index + 1) else {
            return Err(Opaque::UnterminatedQuote);
        };
        if let Some(text) = text.as_deref_mut() {
            text.append(input.slice(index + 1, end));
        }
        return Ok(Some(end));
    }
    if mode == Mode::Arithmetic && char == '\'' {
        return scan_span(
            input,
            index + 1,
            depth,
            ctx,
            commands,
            Span::ARITHMETIC_QUOTE,
            allow_bracket,
            text,
        )
        .map(Some);
    }
    if mode == Mode::Word && opener == Some("$\"") {
        return scan_span(
            input,
            index + 2,
            depth,
            ctx,
            commands,
            Span::DOUBLE,
            allow_bracket,
            text,
        )
        .map(Some);
    }
    if mode != Mode::Quoted && char == '"' {
        let span = if mode == Mode::Arithmetic {
            Span::ARITHMETIC_DOUBLE
        } else {
            Span::DOUBLE
        };
        return scan_span(
            input,
            index + 1,
            depth,
            ctx,
            commands,
            span,
            allow_bracket,
            text,
        )
        .map(Some);
    }
    let end = match opener {
        Some("<(" | ">(") => Some(scan_nested(
            input,
            index + 2,
            depth,
            ctx,
            commands,
            Close::Paren,
            None,
        )?),
        Some(o) if o != "$'" && o != "$\"" => Some(scan_dollar_or_backtick(
            input,
            index,
            depth,
            ctx,
            commands,
            mode != Mode::Word,
            allow_bracket,
            text.as_deref_mut(),
        )?),
        _ if char == '`' => Some(scan_dollar_or_backtick(
            input,
            index,
            depth,
            ctx,
            commands,
            mode != Mode::Word,
            allow_bracket,
            text.as_deref_mut(),
        )?),
        _ => None,
    };
    if end.is_none() && char == '$' && (next.is_ascii_alphanumeric() || "_@*#?$!-".contains(next)) {
        if let Some(text) = text.as_deref_mut() {
            text.word.push('$');
        }
        return Ok(Some(index));
    }
    if let (Some(text), Some(end)) = (text, end) {
        text.word.push_str(input.slice(index, end + 1));
    }
    Ok(end)
}

/// Zsh flag groups such as ${(e)name} evaluate a value, and a modifier run with ~, such as $^~name
/// or ${(f)~name}, globs it, which can run glob qualifier code.
fn zsh_evaluates(input: &Src, index: usize) -> bool {
    let mut at = index + 1;
    if input.starts_with(at, "{\\\n") {
        return true;
    }
    if input.is(at, '{') && input.is(at + 1, '(') {
        let Some(end) = input.sticky(&ZSH_SAFE_FLAGS, index) else {
            return true;
        };
        at = end;
    }
    if input.is(at, '{') {
        at += 1;
    }
    input.sticky(&ZSH_GLOB_MODIFIER, at).is_some()
}

// Arithmetic evaluation expands `name[subscript]` again, so a literal expansion in text the shell evaluates as
// arithmetic can run: (( )), $(( )), $[ ], subscripts, ${name:offset:length}, let operands, and the arithmetic
// operands of [[ ]]. Builtins that name or bind a variable from an operand evaluate its subscript, so a literal
// expansion there runs when the operand also has a bracket; a command substitution's words stand in for the
// output it supplies. Values bound for later, such as assignment values, array elements, for lists, and
// here-strings, reach arithmetic or subscript evaluation when a variable sink appears in the command.
// A parameter's words count as its value. Text that reaches arithmetic only at runtime, through the
// environment, a function argument, or the output a command substitution supplies to a later binding, remains
// out of reach.
fn evaluates_expansion(text: &Text, evaluation: Evaluation, ctx: &Ctx) -> bool {
    let combined = format!("{} {}", text.literal, text.output);
    let has_expansion = EXPANSION.is_match(&combined);
    // Escapes reach the value through $'...' or a substitution's printf, echo -e, or tr.
    let has_escape = ESCAPE.is_match(&combined);
    if has_expansion || has_escape || text.literal.contains(['$', '`']) {
        ctx.bound_dollar.set(true);
    }
    if has_expansion || has_escape || text.literal.contains(['(', '{', '`']) {
        ctx.bound_open.set(true);
    }
    // A substitution's output can hold `[` that the script never spells (printf '\x5b', tr, base64 -d).
    if has_expansion
        || has_escape
        || !super::js_trim(&text.output).is_empty()
        || combined.contains('[')
    {
        ctx.bound_bracket.set(true);
    }
    if evaluation == Evaluation::Deferred {
        return false;
    }
    if text.word.contains('[') || combined.contains('[') {
        ctx.sink_bracket.set(true);
    }
    static ARITHMETIC_VARIABLE: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
        fancy_regex::Regex::new(r"[A-Za-z_]|(?:\$(?![?#!-]|\$(?!\[)))")
            .expect("scanner regex is valid")
    });
    let sink = if evaluation == Evaluation::Arithmetic {
        ARITHMETIC_VARIABLE.is_match(&text.word).unwrap_or(false)
    } else {
        text.word.contains('$') || combined.contains('[')
    };
    if sink {
        ctx.variable_sink.set(true);
    }
    if evaluation == Evaluation::Arithmetic {
        return has_expansion;
    }
    combined.contains('[') && has_expansion
}

/// Scans a subscript after its `[`; indexed arrays evaluate it as arithmetic.
fn scan_subscript(
    input: &Src,
    start: usize,
    depth: usize,
    ctx: &Ctx,
    commands: &mut Vec<Cmd>,
    span: Span,
) -> Step<usize> {
    let mut text = Text::default();
    let end = scan_span(
        input,
        start,
        depth,
        ctx,
        commands,
        span,
        false,
        Some(&mut text),
    )?;
    ctx.sink_bracket.set(true);
    if evaluates_expansion(&text, Evaluation::Arithmetic, ctx) {
        return Err(Opaque::DynamicExecution);
    }
    Ok(end)
}

/// Scans to the span's unnested close character and returns its index; a span without one runs to the end.
#[allow(clippy::too_many_arguments)]
fn scan_span(
    input: &Src,
    start: usize,
    depth: usize,
    ctx: &Ctx,
    commands: &mut Vec<Cmd>,
    span: Span,
    allow_bracket: bool,
    mut text: Option<&mut Text>,
) -> Step<usize> {
    let mut nesting = 0usize;
    let mut index = start;
    while index < input.len() {
        if ctx.spent() {
            return Err(Opaque::InvalidStructure);
        }
        let char = input.chars[index];
        if Some(char) == span.close && nesting == 0 {
            return Ok(index);
        }
        // A `#` after a digit is a base prefix, and after `$` the parameter $#.
        if span.reject.contains(char)
            && !(char == '#'
                && input
                    .before(index)
                    .is_some_and(|c| c.is_ascii_digit() || c == '$'))
        {
            return Err(Opaque::InvalidStructure);
        }
        if Some(char) == span.open {
            nesting += 1;
            if nesting + depth > MAX_SUBSTITUTION_DEPTH {
                return Err(Opaque::InvalidStructure);
            }
        }
        if Some(char) == span.close {
            nesting = nesting.wrapping_sub(1);
        }
        let unit = if Some(char) == span.open || Some(char) == span.close {
            None
        } else {
            scan_unit(
                input,
                index,
                depth,
                ctx,
                commands,
                span.mode,
                allow_bracket,
                text.as_deref_mut(),
            )?
        };
        if let Some(unit) = unit {
            index = unit;
        } else if char == '\\'
            && (input
                .at(index + 1)
                .is_some_and(|n| Some(n) == span.open || Some(n) == span.close))
        {
            if let Some(text) = text.as_deref_mut() {
                text.append(input.slice(index, index + 2));
            }
            index += 1;
        } else if let Some(text) = text.as_deref_mut() {
            let mut buffer = [0; 4];
            text.append(char.encode_utf8(&mut buffer));
        }
        index += 1;
    }
    if span.close.is_some() {
        Err(Opaque::UnterminatedQuote)
    } else {
        Ok(input.len())
    }
}

/// Scans a nested list such as a command or process substitution and returns its closing index.
fn scan_nested(
    input: &Src,
    start: usize,
    depth: usize,
    ctx: &Ctx,
    commands: &mut Vec<Cmd>,
    close: Close,
    text: Option<&mut Text>,
) -> Step<usize> {
    let nested = scan_bash(input, start, depth + 1, ctx, Some(close), None, text)?;
    commands.extend(nested.commands);
    Ok(nested.end)
}

/// Returns the first listed operator at `index`. Shells disagree about operators split by line
/// continuations, so such a split is opaque.
fn operator(input: &Src, index: usize, operators: &[&'static str]) -> Step<Option<&'static str>> {
    let mut text = String::new();
    let mut cursor = index;
    while text.chars().count() < 3 && cursor < input.len() {
        if input.starts_with(cursor, "\\\n") {
            cursor += 1;
        } else {
            text.push(input.chars[cursor]);
        }
        cursor += 1;
    }
    let found = operators.iter().copied().find(|o| text.starts_with(o));
    if let Some(o) = found
        && !input.starts_with(index, o)
    {
        return Err(Opaque::InvalidStructure);
    }
    Ok(found)
}

#[allow(clippy::too_many_arguments)]
fn scan_dollar_or_backtick(
    input: &Src,
    start: usize,
    depth: usize,
    ctx: &Ctx,
    commands: &mut Vec<Cmd>,
    quoted: bool,
    allow_bracket: bool,
    text: Option<&mut Text>,
) -> Step<usize> {
    if depth >= MAX_SUBSTITUTION_DEPTH {
        return Err(Opaque::CommandSubstitution);
    }
    if input.is(start, '`') {
        return scan_backtick(input, start, depth + 1, ctx, commands, quoted, text);
    }
    if input.starts_with(start, "$((")
        && let Some(end) =
            scan_arithmetic(input, start + 3, depth + 1, ctx, commands, Span::ARITHMETIC)?
    {
        return Ok(end);
    }
    if input.starts_with(start, "$(") {
        return scan_nested(input, start + 2, depth, ctx, commands, Close::Paren, text);
    }
    if let Some(after) = input.sticky(&NOFORK_OPEN, start) {
        return scan_nested(input, after, depth, ctx, commands, Close::Nofork, text);
    }
    if input.starts_with(start, "${") {
        // Literal text in parameter words reaches the enclosing word's value.
        let mut parameter = Text::default();
        let end = scan_parameter(
            input,
            start + 2,
            depth + 1,
            ctx,
            commands,
            quoted,
            &mut parameter,
        );
        // ${!name} and Zsh ${(P)name} evaluate the subscript in the referenced name.
        let indirect = input
            .sticky(&LINE_CONTINUATION, start + 2)
            .is_some_and(|at| input.is(at, '!'))
            || (!ctx.bash()
                && input.is(start + 2, '(')
                && input
                    .sticky(&ZSH_SAFE_FLAGS, start)
                    .is_some_and(|flags_end| {
                        ZSH_QUOTED_FLAG_ARG
                            .replace_all(input.slice(start + 3, flags_end - 1), "")
                            .contains('P')
                    }));
        if indirect {
            ctx.variable_sink.set(true);
            if evaluates_expansion(&parameter, Evaluation::Binding, ctx) {
                return Err(Opaque::DynamicExecution);
            }
        }
        if let Some(text) = text {
            text.literal.push_str(&parameter.literal);
            text.output.push_str(&parameter.output);
        }
        return end;
    }
    // Dash leaves `$[` literal, which splits words differently; Bash and Zsh always read arithmetic.
    if !allow_bracket && ctx.dialect == Dialect::Posix {
        return Err(Opaque::CommandSubstitution);
    }
    let mut expression = Text::default();
    let end = scan_span(
        input,
        start + 2,
        depth + 1,
        ctx,
        commands,
        Span::BRACKET_ARITHMETIC,
        true,
        Some(&mut expression),
    )?;
    if evaluates_expansion(&expression, Evaluation::Arithmetic, ctx) {
        return Err(Opaque::DynamicExecution);
    }
    Ok(end)
}

/// Scans the text of ((...)) after its opening parentheses. `None` when the text closes with a
/// single parenthesis, which makes it a nested subshell instead.
fn scan_arithmetic(
    input: &Src,
    start: usize,
    depth: usize,
    ctx: &Ctx,
    commands: &mut Vec<Cmd>,
    span: Span,
) -> Step<Option<usize>> {
    let mut arithmetic = Vec::new();
    let mut text = Text::default();
    let end = scan_span(
        input,
        start,
        depth,
        ctx,
        &mut arithmetic,
        span,
        true,
        Some(&mut text),
    )
    .map_err(|_| Opaque::InvalidStructure)?;
    if operator(input, end, &["))"])?.is_none() {
        return Ok(None);
    }
    if evaluates_expansion(&text, Evaluation::Arithmetic, ctx) {
        return Err(Opaque::DynamicExecution);
    }
    commands.append(&mut arithmetic);
    Ok(Some(end + 1))
}

fn scan_backtick(
    input: &Src,
    start: usize,
    depth: usize,
    ctx: &Ctx,
    commands: &mut Vec<Cmd>,
    quoted: bool,
    text: Option<&mut Text>,
) -> Step<usize> {
    let mut source = String::new();
    let mut index = start + 1;
    while index < input.len() {
        if ctx.spent() {
            return Err(Opaque::InvalidStructure);
        }
        let char = input.chars[index];
        if char == '`' {
            let inner = Src::new(source);
            let inner = scan_bash(&inner, 0, depth, ctx, None, None, text)
                .map_err(|_| Opaque::CommandSubstitution)?;
            commands.extend(inner.commands);
            return Ok(index);
        }
        let next = input.at(index + 1).unwrap_or('\0');
        if char == '\\' && ("$`\\\n".contains(next) || (quoted && next == '"')) {
            index += 1;
            if next != '\n' {
                source.push(next);
            }
            index += 1;
            continue;
        }
        source.push(char);
        index += 1;
    }
    Err(Opaque::CommandSubstitution)
}

fn scan_parameter(
    input: &Src,
    start: usize,
    depth: usize,
    ctx: &Ctx,
    commands: &mut Vec<Cmd>,
    quoted: bool,
    text: &mut Text,
) -> Step<usize> {
    let name_start = input.sticky(&PARAMETER_PREFIX, start).unwrap_or(start);
    // The operands of ${name:offset:length} are arithmetic, and ${name:=word} or ${name=word} binds a variable.
    let mut colon = input.sticky(&PARAMETER_NAME, name_start);
    let name = match colon {
        Some(colon) if colon > name_start => input.slice(name_start, colon).replace("\\\n", ""),
        _ => String::new(),
    };
    let after_name = colon.and_then(|c| input.at(c));
    if !ctx.bash()
        && (name == "NULLCMD"
            || name == "READNULLCMD"
            || (ZSH_SPECIAL_NAME.is_match(&name) && includes("[=:", after_name)))
    {
        return Err(Opaque::DynamicExecution);
    }
    if name == "PS4" {
        ctx.eval_sink.set(true);
    }
    let mut evaluation: Option<Evaluation> = None;
    let mut word_offset = 0;
    let mut literal_offset = 0;
    let mut index = start;
    while index < input.len() {
        if ctx.spent() {
            return Err(Opaque::InvalidStructure);
        }
        let char = input.chars[index];
        if char == '}' {
            if let Some(evaluation) = evaluation {
                let operand = Text {
                    word: text.word[word_offset..].to_string(),
                    literal: text.literal[literal_offset..].to_string(),
                    output: text.output.clone(),
                };
                if evaluates_expansion(&operand, evaluation, ctx) {
                    return Err(Opaque::DynamicExecution);
                }
            }
            return Ok(index);
        }
        if Some(index) == colon && evaluation.is_none() {
            let next = input.at(index + 1);
            if char == ':' && !includes("-=?+", next) {
                evaluation = Some(Evaluation::Arithmetic);
            }
            if char == '=' || (char == ':' && next == Some('=')) {
                evaluation = Some(Evaluation::Deferred);
            }
            if evaluation.is_some() {
                word_offset = text.word.len();
                literal_offset = text.literal.len();
            }
        }
        // Bash ${name@P} evaluates the value as prompt text.
        if char == '@' && input.sticky(&PARAMETER_PROMPT, index).is_some() {
            return Err(Opaque::DynamicExecution);
        }
        // Zsh globs an unquoted parameter's words, where a parenthesized group can be a glob qualifier.
        if !quoted && !ctx.bash() && char == '(' && index > start {
            return Err(Opaque::DynamicExecution);
        }
        if quoted && char == '\'' && input.before(index) == Some('$') {
            let Some(ansi) = ansi_quote(input, index, true) else {
                return Err(Opaque::UnterminatedQuote);
            };
            text.literal.push_str(&ansi.value);
        }
        let subscript = Some(index) == colon && char == '[';
        let unit = if subscript {
            Some(scan_subscript(
                input,
                index + 1,
                depth,
                ctx,
                commands,
                Span::SUBSCRIPT,
            )?)
        } else if quoted && char == '\'' {
            Some(scan_span(
                input,
                index + 1,
                depth,
                ctx,
                commands,
                Span::PARAMETER_QUOTE,
                false,
                Some(text),
            )?)
        } else if quoted && char == '"' {
            Some(scan_span(
                input,
                index + 1,
                depth,
                ctx,
                commands,
                Span::DOUBLE,
                false,
                Some(text),
            )?)
        } else {
            let mode = if quoted { Mode::Quoted } else { Mode::Word };
            scan_unit(input, index, depth, ctx, commands, mode, false, Some(text))?
        };
        // Bash 3.2 ends a double-quoted parameter at an unquoted `}` inside `$(...)`.
        if quoted
            && let Some(unit) = unit
            && input.starts_with(index, "$(")
            && input.slice(index, unit + 1).contains('}')
        {
            return Err(Opaque::CommandSubstitution);
        }
        if let Some(unit) = unit
            && (subscript || (index == name_start && input.starts_with(index, "${")))
        {
            colon = Some(
                input
                    .sticky(&LINE_CONTINUATION, unit + 1)
                    .unwrap_or(unit + 1),
            );
        }
        match unit {
            Some(unit) => index = unit,
            None => {
                let mut buffer = [0; 4];
                text.append(char.encode_utf8(&mut buffer));
            }
        }
        index += 1;
    }
    Err(Opaque::CommandSubstitution)
}

fn scan_conditional(
    input: &Src,
    start: usize,
    depth: usize,
    ctx: &Ctx,
    commands: &mut Vec<Cmd>,
) -> Step<usize> {
    let mut words: Vec<Text> = Vec::new();
    let mut text = Text::default();
    let mut word_started = false;
    let mut has_word = false;
    let mut paren_depth = 0i64;
    let mut pattern_paren_depth = 0i64;
    let flush = |words: &mut Vec<Text>, text: &mut Text, has_word: &mut bool| {
        if *has_word {
            words.push(std::mem::take(text));
            *has_word = false;
        }
    };
    let operator_word = |word: char| Text {
        word: word.to_string(),
        ..Default::default()
    };
    let mut index = start;
    while index < input.len() {
        if ctx.spent() {
            return Err(Opaque::InvalidStructure);
        }
        let char = input.chars[index];
        if char == '\\' && input.is(index + 1, '\n') {
            index += 2;
            continue;
        }
        if !word_started {
            flush(&mut words, &mut text, &mut has_word);
        }
        let close = if !word_started && char == ']' {
            operator(input, index, &["]]"])?
        } else {
            None
        };
        if close.is_some() && input.sticky(&BRACE_CLOSE_AHEAD, index + 2).is_some() {
            if paren_depth != 0 {
                return Err(Opaque::InvalidStructure);
            }
            // -v names a variable, -t in Zsh evaluates the descriptor, and arithmetic comparisons evaluate both operands.
            let word_at =
                |at: Option<usize>| at.and_then(|a| words.get(a)).map(|w| w.word.as_str());
            let evaluated = words.iter().enumerate().any(|(at, word)| {
                let previous = word_at(at.checked_sub(1));
                let following = word_at(Some(at + 1));
                (previous == Some("-v") && evaluates_expansion(word, Evaluation::Binding, ctx))
                    || (((!ctx.bash() && previous == Some("-t"))
                        || previous.is_some_and(|p| ARITHMETIC_COMPARISONS.contains(&p))
                        || following.is_some_and(|f| ARITHMETIC_COMPARISONS.contains(&f)))
                        && evaluates_expansion(word, Evaluation::Arithmetic, ctx))
            });
            if evaluated {
                return Err(Opaque::DynamicExecution);
            }
            return Ok(index + 1);
        }
        if !word_started && char == '#' {
            let Some(newline) = input.index_of('\n', index) else {
                return Err(Opaque::InvalidStructure);
            };
            index = newline + 1;
            continue;
        }
        if matches!(char, ' ' | '\t' | '\n') {
            if pattern_paren_depth == 0 {
                word_started = false;
            }
            index += 1;
            continue;
        }
        if char == ';' {
            return Err(Opaque::InvalidStructure);
        }
        if char == '(' || char == ')' {
            if char == ')' && paren_depth == 0 {
                return Err(Opaque::InvalidStructure);
            }
            let in_pattern = pattern_paren_depth > 0
                || (char == '('
                    && words
                        .last()
                        .is_some_and(|w| ["=", "==", "!=", "=~"].contains(&w.word.as_str())));
            let step = if char == '(' { 1 } else { -1 };
            paren_depth += step;
            if in_pattern {
                pattern_paren_depth += step;
                word_started = true;
                has_word = true;
                text.append(if char == '(' { "(" } else { ")" });
                index += 1;
                continue;
            }
            flush(&mut words, &mut text, &mut has_word);
            words.push(operator_word(char));
            word_started = false;
            index += 1;
            continue;
        }
        let unit = scan_unit(
            input,
            index,
            depth,
            ctx,
            commands,
            Mode::Word,
            true,
            Some(&mut text),
        )?;
        if let Some(unit) = unit {
            index = unit;
        }
        if unit.is_none() && pattern_paren_depth == 0 && "&|<>".contains(char) {
            flush(&mut words, &mut text, &mut has_word);
            words.push(operator_word(char));
            word_started = false;
            index += 1;
            continue;
        }
        word_started = true;
        has_word = true;
        if unit.is_none() {
            let mut buffer = [0; 4];
            text.append(char.encode_utf8(&mut buffer));
        }
        index += 1;
    }
    Err(Opaque::InvalidStructure)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Array,
    Pattern,
}

fn scan_array_or_pattern(
    input: &Src,
    start: usize,
    depth: usize,
    ctx: &Ctx,
    commands: &mut Vec<Cmd>,
    mode: Group,
) -> Step<usize> {
    if depth > MAX_SUBSTITUTION_DEPTH {
        return Err(Opaque::CommandSubstitution);
    }
    // Array elements bind variables, so their decoded text is a subscript sink.
    let mut text = (mode == Group::Array).then(Text::default);
    let mut word_started = false;
    let mut index = start + 1;
    while index < input.len() {
        if ctx.spent() {
            return Err(Opaque::InvalidStructure);
        }
        let char = input.chars[index];
        if char == '\\' && input.is(index + 1, '\n') {
            index += 2;
            continue;
        }
        if let Some(text) = text.as_mut()
            && matches!(char, ')' | ' ' | '\t' | '\n')
        {
            evaluates_expansion(text, Evaluation::Deferred, ctx);
            text.literal.clear();
            text.output.clear();
        }
        if char == ')' {
            return Ok(index);
        }
        if mode == Group::Array && !word_started && char == '#' {
            let Some(newline) = input.index_of('\n', index) else {
                return Err(Opaque::CommandSubstitution);
            };
            index = newline + 1;
            continue;
        }
        if char == ' ' || char == '\t' || (mode == Group::Array && char == '\n') {
            word_started = false;
            index += 1;
            continue;
        }
        if mode == Group::Array
            && word_started
            && char == '('
            && ctx.dialect == Dialect::Zsh
            && let Some(after) = input.sticky(&ZSH_GLOB_QUALIFIER, index)
        {
            let end = after - 1;
            if let Some(text) = text.as_mut() {
                text.literal.push_str(input.slice(index, end + 1));
            }
            index = end + 1;
            continue;
        }
        // Array elements are globbed, where a parenthesized group can be a Zsh glob qualifier.
        if matches!(char, '\n' | ';' | '&')
            || (mode == Group::Array && (char == '|' || char == '('))
        {
            return Err(Opaque::CommandSubstitution);
        }
        word_started = true;
        let unit = if mode == Group::Array && char == '=' && input.is(index + 1, '(') {
            Some(scan_nested(
                input,
                index + 2,
                depth,
                ctx,
                commands,
                Close::Paren,
                None,
            )?)
        } else if char == '(' {
            Some(scan_array_or_pattern(
                input,
                index,
                depth + 1,
                ctx,
                commands,
                mode,
            )?)
        } else if char == '[' {
            Some(scan_subscript(
                input,
                index + 1,
                depth,
                ctx,
                commands,
                Span::SUBSCRIPT,
            )?)
        } else {
            scan_unit(
                input,
                index,
                depth,
                ctx,
                commands,
                Mode::Word,
                false,
                text.as_mut(),
            )?
        };
        match unit {
            Some(unit) => index = unit,
            None => {
                if let Some(text) = text.as_mut() {
                    text.literal.push(char);
                }
            }
        }
        index += 1;
    }
    Err(Opaque::CommandSubstitution)
}

struct Quote {
    value: String,
    end: usize,
}

fn ansi_escape(c: char) -> Option<char> {
    Some(match c {
        'a' => '\x07',
        'b' => '\x08',
        'e' | 'E' => '\x1b',
        'f' => '\x0c',
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        'v' => '\x0b',
        '\\' | '\'' | '"' | '?' => c,
        _ => return None,
    })
}

/// Decodes `'...'` with ANSI-C escapes starting at the quote at `start`.
fn ansi_quote(input: &Src, start: usize, allow_unicode: bool) -> Option<Quote> {
    let mut value = String::new();
    let mut index = start + 1;
    while index < input.len() {
        let char = input.chars[index];
        if char == '\'' {
            return Some(Quote { value, end: index });
        }
        if char != '\\' {
            value.push(char);
            index += 1;
            continue;
        }
        index += 1;
        let escaped = input.at(index)?;
        if let Some(decoded) = ansi_escape(escaped) {
            value.push(decoded);
            index += 1;
            continue;
        }
        let hex = |from: usize, max: usize| -> String {
            (from..from + max)
                .map_while(|i| input.at(i).filter(char::is_ascii_hexdigit))
                .collect()
        };
        let octal = ('0'..='7').contains(&escaped);
        let digits = match escaped {
            'x' => hex(index + 1, 2),
            'u' if allow_unicode => hex(index + 1, 4),
            'U' if allow_unicode => hex(index + 1, 8),
            _ if octal => (index..index + 3)
                .map_while(|i| input.at(i).filter(|c| ('0'..='7').contains(c)))
                .collect(),
            _ => String::new(),
        };
        if digits.is_empty() {
            return None;
        }
        let point = u32::from_str_radix(&digits, if octal { 8 } else { 16 }).ok()?;
        if point == 0 || point > 0x10ffff || (0xd800..=0xdfff).contains(&point) {
            return None;
        }
        value.push(char::from_u32(point)?);
        index += digits.chars().count() - usize::from(octal);
        index += 1;
    }
    None
}

fn heredoc_delimiter(input: &Src, start: usize, tabs: bool) -> Option<Heredoc> {
    let mut delimiter = String::new();
    let mut dash_delimiter: Option<String> = None;
    let mut quoted = false;
    let mut quote: Option<char> = None;
    let mut end = start;
    let mut started = false;
    let done =
        |delimiter: String, dash_delimiter: Option<String>, quoted: bool, end: usize| Heredoc {
            delimiter,
            dash_delimiter,
            quoted,
            tabs,
            end,
            binding: false,
            owned: false,
            command: None,
            start: None,
        };
    let mut index = start;
    while index < input.len() {
        let char = input.chars[index];
        if !started && (char == ' ' || char == '\t') {
            index += 1;
            continue;
        }
        if !started && char == '#' {
            return None;
        }
        if quote.is_none() && " \t\n;&|()<>".contains(char) {
            return started.then(|| done(delimiter, dash_delimiter, quoted, end));
        }
        if quote.is_none() && char == '$' && input.starts_with(index + 1, "\\\n") {
            return None;
        }
        // Dash retains $ in $'...' heredoc delimiters while Bash and Zsh strip $.
        if quote.is_none() && input.starts_with(index, "$'") {
            let literal = ansi_quote(input, index + 1, false)?;
            if Some(literal.end) != input.index_of('\'', index + 2) {
                return None;
            }
            let base = dash_delimiter.take().unwrap_or_else(|| delimiter.clone());
            dash_delimiter = Some(format!("{base}${}", input.slice(index + 2, literal.end)));
            delimiter.push_str(&literal.value);
            quoted = true;
            started = true;
            index = literal.end;
            end = index;
            index += 1;
            continue;
        }
        // Bash strips $ from $"..." heredoc delimiters while Zsh and Dash retain $.
        if quote.is_none() && input.starts_with(index, "$\"") {
            return None;
        }
        if Some(char) == quote {
            quote = None;
            end = index;
            index += 1;
            continue;
        }
        if char == '\\' && quote != Some('\'') {
            let next = input.at(index + 1)?;
            if next == '\n' {
                index += 2;
                continue;
            }
            // Double quotes only remove escapes for shell-special characters.
            if quote.is_none() || "$`\"\\".contains(next) {
                quoted = true;
                started = true;
                index += 1;
                if let Some(dash) = dash_delimiter.as_mut() {
                    dash.push(next);
                }
                delimiter.push(next);
                end = index;
                index += 1;
                continue;
            }
        }
        if quote.is_none() && (char == '\'' || char == '"') {
            quote = Some(char);
            quoted = true;
            started = true;
            end = index;
            index += 1;
            continue;
        }
        if let Some(dash) = dash_delimiter.as_mut() {
            dash.push(char);
        }
        delimiter.push(char);
        started = true;
        end = index;
        index += 1;
    }
    (started && quote.is_none()).then(|| done(delimiter, dash_delimiter, quoted, end))
}

struct Body {
    source: String,
    end: usize,
}

fn heredoc_body(input: &Src, start: usize, delimiter: &Heredoc) -> Option<Body> {
    let body_start = start;
    let mut start = start;
    let mut line_start = start;
    let mut line = String::new();
    // Backslashes ending the joined line; an odd count continues it.
    let mut backslashes = 0usize;
    let max_length = delimiter.delimiter.chars().count().max(
        delimiter
            .dash_delimiter
            .as_ref()
            .map_or(0, |d| d.chars().count()),
    );
    let mut index = start;
    while index <= input.len() {
        if index < input.len() && input.chars[index] != '\n' {
            index += 1;
            continue;
        }
        let raw = input.slice(start, index);
        let text = if delimiter.tabs && start == line_start {
            raw.trim_start_matches('\t')
        } else {
            raw
        };
        let length = text.chars().count();
        let run = text.chars().rev().take_while(|&c| c == '\\').count();
        backslashes = if run == length {
            backslashes + run
        } else {
            run
        };
        let continued = !delimiter.quoted && backslashes % 2 == 1 && index < input.len();
        // Joined lines only grow, so text past the delimiter's length cannot change the comparison.
        if line.chars().count() <= max_length {
            if continued {
                line.push_str(&text[..text.len() - 1]);
            } else {
                line.push_str(text);
            }
        }
        if continued {
            backslashes -= 1;
            start = index + 1;
            index += 1;
            continue;
        }
        if delimiter.dash_delimiter.as_deref() == Some(line.as_str()) {
            return None;
        }
        if line == delimiter.delimiter {
            // Dash does not join backslash-continued delimiter lines; fail closed if a physical delimiter follows.
            if start > line_start && input.find(&delimiter.delimiter, index + 1).is_some() {
                return None;
            }
            return Some(Body {
                source: input.slice(body_start, line_start).to_string(),
                end: index,
            });
        }
        line.clear();
        backslashes = 0;
        start = index + 1;
        line_start = start;
        index += 1;
    }
    None
}
