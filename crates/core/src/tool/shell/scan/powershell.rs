//! PowerShell (`scanPowerShell` in 0.5.1's `shell/scan.ts`).

use std::collections::HashSet;
use std::sync::LazyLock;

use super::{
    Command, MAX_INPUT_LENGTH, MAX_SUBSTITUTION_DEPTH, Opaque, Src, js_space, js_trim, js_trim_end,
    ps_word_char,
};

type Step<T> = Result<T, Opaque>;
type Skipped = HashSet<usize>;

macro_rules! sticky_re {
    ($name:ident, $pattern:expr) => {
        static $name: LazyLock<fancy_regex::Regex> = LazyLock::new(|| super::sticky($pattern));
    };
}

sticky_re!(STOP_PARSING, r"--%(?=$|[\s;|&(){}])");
sticky_re!(MERGE_REDIRECT, r"\d>&");
sticky_re!(
    LABEL,
    r"(?i):[A-Za-z_]\w*(?=\s*(?:while|for|foreach|do|switch)(?=$|[\s&(),;{|}]))"
);
sticky_re!(KEYWORD, r"[A-Za-z]+(?=$|[\s&(),;{|}])");
sticky_re!(FOREACH, r"(?i)foreach\s*\(");
sticky_re!(PARAM, r"(?i)param\s*\(");
sticky_re!(IN, r"(?i)in(?=$|[\s&(),;{|}])");
sticky_re!(HERE_HEADER, r#"@['"][^\S\r\n]*(?:\r\n|\r|\n)"#);
sticky_re!(UNICODE_ESCAPE, r"(?i)u\{([0-9a-f]{1,6})\}");
sticky_re!(
    EXPRESSION,
    r"(?i)(?:!*(?:\+\+|--|[+-])?\$(?:\{|(?:\?|\$|\^)(?=$|[\s;|&(){},#>])|:[\p{L}\p{Nl}\d_?]+|[\p{L}\p{Nl}\d_][\p{L}\p{Nl}\d_?]*(?::[\p{L}\p{Nl}\d_?]+)?)|!(?![\p{L}\p{Nl}_.\d])|\+(?![=\p{L}\p{Nl}_.\d])|--(?!=)|-(?![=\p{L}\p{Nl}\d_?.])|,(?!\s*$)|!*[+-]?(?:0[xX][\da-fA-F]+|0[bB][01]+|(?:\d+(?:\.(?!\.)\d*|(?!\.(?!\.)))|\.\d+)(?:[eE][+-]?\d+)?)(?:[dDlLnN]|[uU][sSyYlL]?|[sSyY])?(?:[kKmMgGtTpP][bB])?(?=$|[\s!#%&()*+,\-/.;<=>\]|{}])|-(?:not|bnot|join|[ic]?split)(?![\p{L}\p{Nl}]))"
);
static PARAMETER_COLON: LazyLock<regex::Regex> =
    LazyLock::new(|| super::regex(r"^-[\p{L}\p{Nl}_?][\p{L}\p{Nl}0-9_?-]*:$"));
static TYPE_NAME: LazyLock<regex::Regex> =
    LazyLock::new(|| super::regex(r"^[\p{L}\p{Nl}_][\p{L}\p{Nl}0-9_.#+`\\]*$"));
static BLOCK_KEYWORD: LazyLock<regex::Regex> = LazyLock::new(|| {
    super::regex(
        r"^(?:if|for|while|do|switch|function|filter|workflow|parallel|sequence|configuration|try|begin|process|end|clean|trap|class|enum|data|dynamicparam|using|break|continue)$",
    )
});

pub fn scan_powershell(input: &str) -> Result<Vec<Command>, Opaque> {
    if super::utf16_len(input) > MAX_INPUT_LENGTH {
        return Err(Opaque::InvalidStructure);
    }
    // PowerShell's Unicode quotes, dashes, and whitespace differ from JavaScript's token rules.
    if input.contains(|c| {
        matches!(c, '\0' | '\u{85}' | '\u{2013}'..='\u{2015}' | '\u{2018}'..='\u{201e}' | '\u{feff}')
    }) {
        return Err(Opaque::InvalidStructure);
    }
    let src = Src::new(input.to_string());
    let mut budget = (MAX_INPUT_LENGTH * MAX_SUBSTITUTION_DEPTH) as i64;
    let scanned = scan_list(&src, 0, 0, &mut budget, None, Hash::No, None)?;
    Ok(scanned.commands)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Clause {
    Member,
    Block,
    If,
    Try,
    Class,
    Switch,
    Paren,
    Do,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hash {
    No,
    Yes,
    Clause,
    Switch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    Do,
    If,
    Try,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Indexable {
    No,
    Variable,
    Closed,
    Member,
}

struct Scanned {
    commands: Vec<Command>,
    end: usize,
}

fn spent(budget: &mut i64) -> bool {
    *budget -= 1;
    *budget < 0
}

fn space_at(input: &Src, index: usize) -> bool {
    input.at(index).is_some_and(js_space)
}

fn ends_with_member(word: &str) -> bool {
    word.ends_with(['.', ':'])
}

struct List<'a> {
    input: &'a Src,
    initial_expression: bool,
    initial_clause: Option<Clause>,
    commands: Vec<Command>,
    nested: Vec<Command>,
    words: Vec<String>,
    raw_words: Vec<String>,
    word_ends: Vec<usize>,
    segment: usize,
    word: String,
    started: bool,
    word_start: usize,
    quote: Option<char>,
    standalone: bool,
    expression: bool,
    clause: Option<Clause>,
    after: Option<After>,
    indexable: Indexable,
    member_chain_start: Option<usize>,
    has_member_access: bool,
    has_file_redirect: bool,
    stop_parsing: bool,
    command_end: usize,
    statement_head: bool,
    invalid: bool,
    redirect_target: bool,
    dangling: bool,
    invocation: bool,
}

impl List<'_> {
    fn reset_member(&mut self) {
        self.indexable = Indexable::No;
        self.member_chain_start = None;
        self.has_member_access = false;
    }

    /// The word a member operator would follow: the current one, else the last finished one.
    fn previous(&self) -> &str {
        if self.started {
            &self.word
        } else {
            self.words.last().map_or("", String::as_str)
        }
    }

    fn member_operator(&self) -> bool {
        self.indexable == Indexable::Member && ends_with_member(self.previous())
    }

    fn finish_word(&mut self, end: usize) {
        if !self.started {
            return;
        }
        // Generic tokens can spell stop-parsing with escapes or embedded quotes; literal strings cannot.
        let last_raw = self.raw_words.last().map_or("", String::as_str);
        if self.word == "--%"
            && !self.words.is_empty()
            && !self.expression
            && self.clause.is_none()
            && !self.redirect_target
            && !matches!(self.input.at(self.word_start), Some('\'' | '"'))
            && !last_raw.ends_with(',')
            && !PARAMETER_COLON.is_match(last_raw)
        {
            self.stop_parsing = true;
        }
        if !self.redirect_target {
            if self.words.is_empty() && !self.invocation {
                self.segment = self.word_start;
            }
            self.words.push(std::mem::take(&mut self.word));
            self.raw_words
                .push(self.input.slice(self.word_start, end).to_string());
            self.word_ends.push(end);
            self.dangling = false;
        }
        self.command_end = end;
        self.redirect_target = false;
        self.word.clear();
        self.started = false;
    }

    fn finish_command(&mut self, end: usize, required: bool) {
        self.finish_word(end);
        let plain = !self.expression && self.clause.is_none();
        if !self.words.is_empty() && (plain || self.has_file_redirect) {
            let segment = self.segment;
            self.commands.push(Command {
                resource: js_trim_end(self.input.slice(segment, self.command_end)).to_string(),
                words: self.words.clone(),
                raw_words: self.raw_words.clone(),
                word_ends: Some(
                    self.word_ends
                        .iter()
                        .map(|e| e.saturating_sub(segment))
                        .collect(),
                ),
                statement_head: self.statement_head && !self.invocation && plain,
                ..Default::default()
            });
        } else if (plain && self.invocation)
            || (required && self.words.is_empty() && self.nested.is_empty())
            || self.clause.is_some_and(|c| c != Clause::Member)
        {
            self.invalid = true;
        }
        if self.redirect_target {
            self.invalid = true;
        }
        self.commands.append(&mut self.nested);
        self.words.clear();
        self.raw_words.clear();
        self.word_ends.clear();
        self.redirect_target = false;
        self.has_file_redirect = false;
        self.invocation = false;
        self.expression = self.initial_expression;
        self.clause = self.initial_clause;
        self.after = None;
        self.reset_member();
        self.stop_parsing = false;
    }
}

fn scan_list(
    input: &Src,
    start: usize,
    depth: usize,
    budget: &mut i64,
    close: Option<char>,
    hash: Hash,
    skipped: Option<&Skipped>,
) -> Step<Scanned> {
    if depth >= MAX_SUBSTITUTION_DEPTH || *budget < 0 {
        return Err(Opaque::InvalidStructure);
    }
    let initial_expression = matches!(hash, Hash::Yes | Hash::Clause);
    let initial_clause = matches!(hash, Hash::Clause | Hash::Switch).then_some(Clause::Member);
    let mut s = List {
        input,
        initial_expression,
        initial_clause,
        commands: Vec::new(),
        nested: Vec::new(),
        words: Vec::new(),
        raw_words: Vec::new(),
        word_ends: Vec::new(),
        segment: start,
        word: String::new(),
        started: false,
        word_start: start,
        quote: None,
        standalone: false,
        expression: initial_expression,
        clause: initial_clause,
        after: None,
        indexable: Indexable::No,
        member_chain_start: None,
        has_member_access: false,
        has_file_redirect: false,
        stop_parsing: false,
        command_end: start,
        statement_head: true,
        invalid: false,
        redirect_target: false,
        dangling: false,
        invocation: false,
    };
    let is_skipped = |index: usize| skipped.is_some_and(|set| set.contains(&index));
    let mut index = start;
    'scan: while index < input.len() {
        'next: {
            if spent(budget) {
                return Err(Opaque::InvalidStructure);
            }
            if is_skipped(index) {
                break 'next;
            }
            let char = input.chars[index];
            let next = input.at(index + 1);
            let in_expression = s.expression && !s.redirect_target;
            let at_boundary = !s.started || (!in_expression && s.word.ends_with(','));
            if !s.started {
                s.word_start = index;
            }
            if s.stop_parsing {
                let stop = stop_parsing_end(input, index, skipped);
                let raw = input.slice(index, stop);
                let text = js_trim(raw);
                if !text.is_empty() {
                    s.word_start = index + raw.chars().take_while(|&c| js_space(c)).count();
                    s.word = text.to_string();
                    s.started = true;
                    let end = s.word_start + text.chars().count();
                    s.finish_word(end);
                }
                s.stop_parsing = false;
                index = stop.wrapping_sub(1);
                break 'next;
            }
            if let Some(quote) = s.quote {
                let mut next_quote = index + 1;
                while is_skipped(next_quote) {
                    next_quote += 1;
                }
                if char == quote && input.at(next_quote) == Some(quote) {
                    s.word.push(quote);
                    index = next_quote;
                } else if char == quote {
                    s.quote = None;
                    if s.standalone {
                        s.finish_word(index + 1);
                        s.indexable = Indexable::Closed;
                    }
                } else if quote == '"' && char == '`' {
                    let Some(escape) = escape(input, index, false, skipped) else {
                        return Err(Opaque::UnterminatedEscape);
                    };
                    s.word.push_str(&escape.value);
                    index = escape.end;
                } else if quote == '"' && char == '$' && next == Some('{') {
                    let Some(end) = braced_variable(input, index, skipped) else {
                        return Err(Opaque::InvalidStructure);
                    };
                    s.word.push_str(input.slice(index, end + 1));
                    index = end;
                } else if quote == '"' && char == '$' && next == Some('(') {
                    let end = sub_list(
                        input,
                        index + 2,
                        depth,
                        budget,
                        false,
                        skipped,
                        &mut s.nested,
                    )?;
                    s.word.push_str(input.slice(index, end + 1));
                    index = end;
                } else {
                    s.word.push(char);
                }
                break 'next;
            }
            if char == '\'' || char == '"' {
                if !s.started && s.words.is_empty() && !s.invocation {
                    s.expression = true;
                }
                s.quote = Some(char);
                let dynamic_member = s.member_operator();
                s.standalone =
                    at_boundary || (s.expression && !s.redirect_target) || dynamic_member;
                if dynamic_member {
                    s.has_member_access = true;
                } else if s.standalone {
                    s.member_chain_start = Some(index);
                    s.has_member_access = false;
                } else {
                    s.reset_member();
                }
                s.started = true;
                break 'next;
            }
            if char == '`' {
                let Some(escape) = escape(input, index, at_boundary || in_expression, skipped)
                else {
                    return Err(Opaque::UnterminatedEscape);
                };
                let raw_space = space_at(input, index + 1);
                if (in_expression || at_boundary) && s.started && raw_space {
                    s.finish_word(index);
                } else if !at_boundary || !raw_space {
                    // At a token boundary escaped whitespace is trivia, not a new argument.
                    s.started = true;
                    if !(s.indexable == Indexable::Member && escape.value.chars().any(ps_word_char))
                    {
                        s.reset_member();
                    }
                    s.word.push_str(&escape.value);
                }
                index = escape.end;
                break 'next;
            }
            if !s.started
                && !s.words.is_empty()
                && !s.expression
                && s.clause.is_none()
                && !s.redirect_target
                && input.sticky(&STOP_PARSING, index).is_some()
            {
                s.word = "--%".into();
                s.started = true;
                s.finish_word(index + 3);
                index += 2;
                break 'next;
            }
            if char == '<'
                && next == Some('#')
                && (at_boundary || in_expression || s.indexable == Indexable::Member)
            {
                let member_operator = s.member_operator();
                if s.started {
                    s.finish_word(index);
                }
                let Some(end) = input.find("#>", index + 2) else {
                    return Err(Opaque::InvalidStructure);
                };
                if !member_operator {
                    s.reset_member();
                }
                index = end + 1;
                break 'next;
            }
            if char == '#'
                && (at_boundary
                    || s.indexable == Indexable::Member
                    || (in_expression && !(close == Some(']') && TYPE_NAME.is_match(&s.word))))
            {
                let after_comma = s.previous().ends_with(',');
                let member_operator = s.member_operator();
                if s.clause.is_some_and(|c| c != Clause::Member) || after_comma || member_operator {
                    s.finish_word(index);
                } else if !s.words.is_empty() || s.started || s.invocation {
                    s.finish_command(index, false);
                }
                s.statement_head = !s.dangling;
                let newline = [input.index_of('\n', index), input.index_of('\r', index)]
                    .into_iter()
                    .flatten()
                    .min();
                let Some(newline) = newline else {
                    index = input.len();
                    break 'scan;
                };
                index = if input.is(newline, '\r') && input.is(newline + 1, '\n') {
                    newline + 1
                } else {
                    newline
                };
                break 'next;
            }
            if in_expression && s.started && (char == '>' || (char == '*' && next == Some('>'))) {
                s.finish_word(index);
            }
            let redirect = if !s.started
                && (char == '>'
                    || char == '*'
                    || (char.is_ascii_digit()
                        && (!s.words.is_empty() || input.sticky(&MERGE_REDIRECT, index).is_some())))
            {
                redirect(input, index)
            } else {
                None
            };
            if let Some(redirect) = redirect {
                let Some(redirect) = redirect else {
                    return Err(Opaque::InvalidRedirect);
                };
                if s.redirect_target
                    || s.words.is_empty()
                    || s.clause.is_some_and(|c| c != Clause::Member)
                {
                    return Err(Opaque::InvalidRedirect);
                }
                s.redirect_target = !redirect.contains('&');
                if s.redirect_target {
                    s.has_file_redirect = true;
                }
                s.reset_member();
                index += redirect.chars().count() - 1;
                s.command_end = index + 1;
                break 'next;
            }
            if !s.started
                && s.words.is_empty()
                && !s.invocation
                && !s.expression
                && s.clause.is_none()
                && char == ':'
                && let Some(end) = input.sticky(&LABEL, index)
            {
                index = end - 1;
                break 'next;
            }
            if !s.started
                && !s.invocation
                && s.clause.is_none()
                && (if s.words.is_empty() {
                    !s.expression
                } else {
                    s.words.iter().all(|w| w.starts_with('['))
                })
                && char.is_ascii_alphabetic()
            {
                let keyword = input
                    .sticky(&KEYWORD, index)
                    .map(|end| input.slice(index, end).to_lowercase());
                if let Some(keyword) = keyword.as_deref() {
                    let param = keyword == "param" && input.sticky(&PARAM, index).is_some();
                    let opens = if !s.words.is_empty() {
                        param || keyword == "class" || keyword == "enum"
                    } else {
                        param
                            || (keyword == "foreach" && input.sticky(&FOREACH, index).is_some())
                            || (keyword == "until" && s.after == Some(After::Do))
                            || ((keyword == "else" || keyword == "elseif")
                                && s.after == Some(After::If))
                            || ((keyword == "catch" || keyword == "finally")
                                && s.after == Some(After::Try))
                            || BLOCK_KEYWORD.is_match(keyword)
                    };
                    if opens {
                        s.expression = matches!(keyword, "class" | "enum" | "break" | "continue");
                        s.clause = Some(match keyword {
                            "using" | "break" | "continue" => Clause::Member,
                            "param" | "until" => Clause::Paren,
                            "while" if s.after == Some(After::Do) => Clause::Paren,
                            "do" => Clause::Do,
                            "if" | "elseif" => Clause::If,
                            "try" | "catch" => Clause::Try,
                            "switch" => Clause::Switch,
                            "class" | "enum" => Clause::Class,
                            _ => Clause::Block,
                        });
                        s.after = None;
                        index += keyword.len() - 1;
                        break 'next;
                    }
                    if s.words.is_empty() && matches!(keyword, "return" | "throw" | "exit") {
                        s.after = None;
                        index += keyword.len() - 1;
                        break 'next;
                    }
                }
            }
            let assign = in_expression
                && s.clause.is_none()
                && (char == '=' || ("-+*/%".contains(char) && next == Some('=')));
            if assign
                || (in_expression
                    && close == Some(')')
                    && !s.started
                    && input.sticky(&IN, index).is_some())
            {
                if assign {
                    s.finish_word(index);
                }
                s.words.clear();
                s.raw_words.clear();
                s.word_ends.clear();
                s.expression = false;
                s.has_file_redirect = false;
                s.reset_member();
                if !assign || char != '=' {
                    index += 1;
                }
                break 'next;
            }
            if at_boundary
                && char == '@'
                && !next.is_some_and(|n| "({'\"?".contains(n) || ps_word_char(n))
            {
                return Err(Opaque::InvalidStructure);
            }
            if at_boundary && char == '@' && matches!(next, Some('\'' | '"')) {
                if s.started {
                    s.finish_word(index);
                }
                s.word_start = index;
                let literal = here_string(input, index, depth, budget, skipped)?;
                if s.words.is_empty() && !s.invocation {
                    s.expression = true;
                }
                s.nested.extend(literal.commands);
                s.word = literal.source;
                s.started = true;
                index = literal.end;
                s.finish_word(index + 1);
                s.member_chain_start = Some(s.word_start);
                s.has_member_access = false;
                s.indexable = Indexable::Closed;
                break 'next;
            }
            if char == '$' && next == Some('{') {
                let Some(end) = braced_variable(input, index, skipped) else {
                    return Err(Opaque::InvalidStructure);
                };
                if !s.started && s.words.is_empty() && !s.invocation && hash != Hash::Switch {
                    s.expression = true;
                }
                let dynamic_member = s.member_operator();
                if dynamic_member {
                    s.indexable = Indexable::Closed;
                    s.has_member_access = true;
                } else if at_boundary {
                    s.member_chain_start = Some(index);
                    s.has_member_access = false;
                    s.indexable = Indexable::Closed;
                } else {
                    s.reset_member();
                }
                s.word.push_str(input.slice(index, end + 1));
                s.started = true;
                index = end;
                if s.expression && !s.redirect_target && !input.is(index + 1, '[') {
                    s.finish_word(index + 1);
                }
                break 'next;
            }
            let opener = if ((char == '$' || (at_boundary && char == '@')) && next == Some('('))
                || (at_boundary && char == '@' && next == Some('{'))
            {
                Some(index + 1)
            } else if char == '('
                || char == '{'
                || (char == '['
                    && (in_expression
                        || s.indexable != Indexable::No
                        || (!s.started
                            && !s.invocation
                            && (s.words.is_empty()
                                || matches!(s.clause, Some(Clause::Block | Clause::Try))))))
            {
                Some(index)
            } else {
                None
            };
            if let Some(opener) = opener {
                let token_index = index;
                let open_char = input.chars[opener];
                let dynamic_member_opener = s.member_operator();
                let prior_indexable = s.indexable != Indexable::No;
                let is_method_call = char == '('
                    && opener == token_index
                    && s.member_chain_start.is_some()
                    && ((s.indexable == Indexable::Member
                        && s.started
                        && !ends_with_member(&s.word))
                        || (s.indexable == Indexable::Closed && !s.started && s.has_member_access));
                let standalone_block =
                    at_boundary || in_expression || char == '(' || open_char == '{';
                let block_close = match open_char {
                    '(' => ')',
                    '[' => ']',
                    _ => '}',
                };
                let block_hash = if open_char == '[' || (char == '@' && open_char == '{') {
                    Hash::Yes
                } else if s.clause == Some(Clause::Switch) && open_char == '{' {
                    Hash::Switch
                } else if (s.clause == Some(Clause::Class) && open_char == '{')
                    || (close == Some(']') && char == '(' && s.started)
                {
                    Hash::Clause
                } else {
                    Hash::No
                };
                if s.started && (char == '{' || char == '(' || (char == '@' && at_boundary)) {
                    s.finish_word(index);
                    if s.stop_parsing {
                        index = index.wrapping_sub(1);
                        break 'next;
                    }
                    s.word_start = index;
                }
                if !s.started && s.words.is_empty() && !s.invocation {
                    s.expression = true;
                }
                let result = scan_list(
                    input,
                    opener + 1,
                    depth + 1,
                    budget,
                    Some(block_close),
                    block_hash,
                    skipped,
                )?;
                s.nested.extend(result.commands);
                s.started = true;
                s.word.push_str(input.slice(index, result.end + 1));
                index = result.end;
                if is_method_call && let Some(chain_start) = s.member_chain_start {
                    s.finish_word(index + 1);
                    let resource = input.slice(chain_start, result.end + 1).to_string();
                    let head = input.slice(chain_start, opener).to_string();
                    let args = input.slice(opener, result.end + 1).to_string();
                    let ends = vec![head.chars().count(), resource.chars().count()];
                    s.nested.push(Command {
                        resource,
                        words: vec![head.clone(), args.clone()],
                        raw_words: vec![head, args],
                        word_ends: Some(ends),
                        ..Default::default()
                    });
                    s.indexable = Indexable::Closed;
                    s.has_member_access = false;
                } else if open_char == '[' {
                    s.indexable = Indexable::Closed;
                    if !prior_indexable {
                        s.member_chain_start = Some(token_index);
                    }
                    if !input.is(index + 1, '[') {
                        s.finish_word(index + 1);
                    }
                } else if standalone_block {
                    s.finish_word(index + 1);
                    s.indexable = Indexable::Closed;
                    if !dynamic_member_opener {
                        s.member_chain_start = Some(token_index);
                    }
                    s.has_member_access = dynamic_member_opener;
                }
                let completes = match s.clause {
                    Some(Clause::Paren) => char == '(',
                    Some(_) => char == '{',
                    None => false,
                };
                if completes {
                    let completed = match s.clause {
                        Some(Clause::Do) => Some(After::Do),
                        Some(Clause::If) => Some(After::If),
                        Some(Clause::Try) => Some(After::Try),
                        _ => None,
                    };
                    s.clause = Some(Clause::Member);
                    s.finish_command(index + 1, false);
                    s.after = completed;
                }
                break 'next;
            }
            if Some(char) == close {
                break 'scan;
            }
            if char == '}' || char == ')' || (char == ']' && in_expression) {
                return Err(Opaque::InvalidStructure);
            }
            if !s.started
                && s.words.is_empty()
                && ((char == '&' && next != Some('&'))
                    || (char == '.'
                        && next.is_none_or(|n| js_space(n) || "\"'$(),;{|}&".contains(n))))
            {
                if s.invocation {
                    return Err(Opaque::InvalidStructure);
                }
                s.segment = index;
                s.invocation = true;
                s.after = None;
                break 'next;
            }
            if js_space(char) && char != '\n' && char != '\r' {
                let member_operator = s.member_operator();
                s.finish_word(index);
                if !member_operator {
                    s.reset_member();
                }
                break 'next;
            }
            let separator = match (char, next) {
                ('\r', Some('\n')) => Some("\r\n"),
                ('&', Some('&')) => Some("&&"),
                ('|', Some('|')) => Some("||"),
                (';', _) => Some(";"),
                ('|', _) => Some("|"),
                ('&', _) => Some("&"),
                ('\n', _) => Some("\n"),
                ('\r', _) => Some("\r"),
                _ => None,
            };
            if let Some(separator) = separator {
                let line = matches!(separator, "\n" | "\r" | "\r\n");
                if close == Some(']') && !line {
                    return Err(Opaque::InvalidStructure);
                }
                if line {
                    let member_operator = s.member_operator();
                    if s.clause.is_some_and(|c| c != Clause::Member)
                        || s.previous().ends_with(',')
                        || member_operator
                    {
                        s.finish_word(index);
                        index += separator.len() - 1;
                        if !member_operator {
                            s.reset_member();
                        }
                        break 'next;
                    }
                    if !s.started && s.words.is_empty() && !s.invocation {
                        index += separator.len() - 1;
                        s.reset_member();
                        break 'next;
                    }
                }
                let required = s.dangling || !(separator == ";" || line);
                s.finish_command(index, required);
                s.dangling = !(separator == ";" || separator == "&" || line);
                s.statement_head = !s.dangling;
                index += separator.len() - 1;
                break 'next;
            }
            if !s.started
                && s.words.is_empty()
                && !s.invocation
                && hash != Hash::Switch
                && input.sticky(&EXPRESSION, index).is_some()
            {
                s.expression = true;
            }
            let after_member = s.member_operator();
            let next_space = space_at(input, index + 1);
            if (at_boundary || after_member) && char == '$' {
                if indexable_variable(input, index + 1) {
                    if after_member {
                        s.has_member_access = true;
                    } else {
                        s.member_chain_start = Some(index);
                        s.has_member_access = false;
                    }
                    s.indexable = Indexable::Variable;
                } else {
                    s.reset_member();
                }
            } else if s.indexable == Indexable::Closed {
                s.indexable =
                    if (char == '.' && next != Some('.') && (!next_space || in_expression))
                        || (!s.started && char == ':' && next == Some(':'))
                        || (!s.started
                            && char == '?'
                            && next == Some('.')
                            && !input.is(index + 2, '.')
                            && (!space_at(input, index + 2) || in_expression))
                    {
                        Indexable::Member
                    } else if !s.started && char == '?' && next == Some('[') {
                        Indexable::Closed
                    } else {
                        Indexable::No
                    };
                if s.indexable == Indexable::Member {
                    s.has_member_access = true;
                } else if s.indexable == Indexable::No {
                    s.reset_member();
                }
            } else if s.indexable == Indexable::Variable {
                s.indexable =
                    if (char == '.' && next != Some('.') && (!next_space || in_expression))
                        || (char == ':' && next == Some(':'))
                    {
                        Indexable::Member
                    } else if (char == '.' && next == Some('.'))
                        || !(ps_word_char(char) || char == ':' || char == '?')
                    {
                        Indexable::No
                    } else {
                        Indexable::Variable
                    };
                if s.indexable == Indexable::Member {
                    s.has_member_access = true;
                } else if s.indexable == Indexable::No {
                    s.reset_member();
                }
            } else if s.indexable == Indexable::Member {
                s.indexable = if char == '.' && next == Some('.') {
                    Indexable::No
                } else if (char == '.' && (!next_space || in_expression))
                    || (char == ':' && (next == Some(':') || input.before(index) == Some(':')))
                    || (char == '?' && next == Some('.') && !input.is(index + 2, '.'))
                    || ps_word_char(char)
                {
                    Indexable::Member
                } else {
                    Indexable::No
                };
                if s.indexable == Indexable::No {
                    s.reset_member();
                }
            }
            s.after = None;
            s.started = true;
            s.dangling = false;
            s.word.push(char);
        }
        index = index.wrapping_add(1);
    }

    if close.is_some() && input.at(index) != close {
        return Err(Opaque::InvalidStructure);
    }
    if s.quote.is_some() {
        return Err(Opaque::UnterminatedQuote);
    }
    s.finish_command(index, false);
    if s.redirect_target || s.invalid || s.dangling {
        return Err(Opaque::InvalidStructure);
    }
    if s.commands
        .iter()
        .any(|c| c.words.first().is_none_or(String::is_empty))
    {
        return Err(Opaque::DynamicCommandName);
    }
    Ok(Scanned {
        commands: s.commands,
        end: index,
    })
}

/// `$(...)` inside a double-quoted string or here-string starting after `$(`: its commands go to
/// `nested`; returns the closing index.
fn sub_list(
    input: &Src,
    start: usize,
    depth: usize,
    budget: &mut i64,
    here_string: bool,
    skipped: Option<&Skipped>,
    nested: &mut Vec<Command>,
) -> Step<usize> {
    let Some(sub) = sub_expression(input, start, here_string, budget, skipped) else {
        return Err(Opaque::InvalidStructure);
    };
    let result = scan_list(
        input,
        start,
        depth + 1,
        budget,
        Some(')'),
        Hash::No,
        Some(&sub.skipped),
    )?;
    nested.extend(result.commands);
    Ok(match sub.raw_end {
        Some(raw_end) if raw_end > result.end => raw_end,
        _ => result.end,
    })
}

/// `$` followed by `:name` or a name (`[\p{L}\p{Nl}\d_]`).
fn indexable_variable(input: &Src, index: usize) -> bool {
    match input.at(index) {
        Some(':') => input
            .at(index + 1)
            .is_some_and(|c| ps_word_char(c) || c == '?'),
        Some(c) => ps_word_char(c),
        None => false,
    }
}

struct Escape {
    value: String,
    end: usize,
}

fn escape(
    input: &Src,
    start: usize,
    line_continuation: bool,
    skipped: Option<&Skipped>,
) -> Option<Escape> {
    let mut next = start + 1;
    while skipped.is_some_and(|set| set.contains(&next)) {
        next += 1;
    }
    let char = input.at(next)?;
    if char == '\r' || char == '\n' {
        let pair = line_continuation && char == '\r' && input.is(next + 1, '\n');
        return Some(Escape {
            value: if pair {
                "\r\n".into()
            } else {
                char.to_string()
            },
            end: next + usize::from(pair),
        });
    }
    if char == 'u' && input.is(next + 1, '{') {
        let end = input.sticky(&UNICODE_ESCAPE, next)?;
        let digits = input.slice(next + 2, end - 1);
        let point = u32::from_str_radix(digits, 16)
            .ok()
            .filter(|&p| p <= 0x10ffff)?;
        return Some(Escape {
            value: char::from_u32(point).unwrap_or('\u{FFFD}').to_string(),
            end: end - 1,
        });
    }
    let value = match char {
        '0' => '\0',
        'a' => '\x07',
        'b' => '\x08',
        'e' => '\x1b',
        'f' => '\x0c',
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        'v' => '\x0b',
        other => other,
    };
    Some(Escape {
        value: value.to_string(),
        end: next,
    })
}

fn braced_variable(input: &Src, start: usize, skipped: Option<&Skipped>) -> Option<usize> {
    let mut index = start + 2;
    while index < input.len() {
        if skipped.is_some_and(|set| set.contains(&index)) {
            index += 1;
            continue;
        }
        if input.is(index, '`') {
            index = escape(input, index, false, skipped)?.end + 1;
            continue;
        }
        if input.is(index, '}') {
            return (index > start + 2).then_some(index);
        }
        index += 1;
    }
    None
}

struct SubExpression {
    skipped: Skipped,
    raw_end: Option<usize>,
}

fn sub_expression(
    input: &Src,
    start: usize,
    here_string: bool,
    budget: &mut i64,
    skipped: Option<&Skipped>,
) -> Option<SubExpression> {
    let mut next_skipped = skipped.cloned().unwrap_or_default();
    let mut depth = 1i64;
    let mut raw_end = None;
    let mut index = start;
    while index < input.len() {
        if spent(budget) {
            return None;
        }
        if next_skipped.contains(&index) {
            index += 1;
            continue;
        }
        let char = input.chars[index];
        if char == '(' {
            depth += 1;
        } else if char == ')' {
            depth -= 1;
            if depth == 0 && raw_end.is_none() {
                raw_end = Some(index);
            }
        } else if !here_string && (char == '"' || char == '`') {
            let mut next = index + 1;
            while next_skipped.contains(&next) {
                next += 1;
            }
            if input.is(next, '"') {
                next_skipped.insert(index);
                index = next;
            }
        }
        index += 1;
    }
    Some(SubExpression {
        skipped: next_skipped,
        raw_end,
    })
}

fn stop_parsing_end(input: &Src, start: usize, skipped: Option<&Skipped>) -> usize {
    let mut quoted = false;
    for index in start..input.len() {
        if skipped.is_some_and(|set| set.contains(&index)) {
            continue;
        }
        let char = input.chars[index];
        if char == '"' {
            quoted = !quoted;
        }
        if char == '\r'
            || char == '\n'
            || ((char == '|' || (char == '&' && input.is(index + 1, '&'))) && !quoted)
        {
            return index;
        }
    }
    input.len()
}

struct HereString {
    commands: Vec<Command>,
    source: String,
    end: usize,
}

fn here_string(
    input: &Src,
    start: usize,
    depth: usize,
    budget: &mut i64,
    skipped: Option<&Skipped>,
) -> Step<HereString> {
    let Some(body) = input.sticky(&HERE_HEADER, start) else {
        return Err(Opaque::UnterminatedQuote);
    };
    let quote = input.chars[start + 1];
    let mut commands = Vec::new();
    let mut line_start = true;
    let mut index = body;
    while index < input.len() {
        if spent(budget) {
            return Err(Opaque::InvalidStructure);
        }
        if skipped.is_some_and(|set| set.contains(&index)) {
            index += 1;
            continue;
        }
        let char = input.chars[index];
        if line_start && char == quote && input.is(index + 1, '@') {
            let end = if index > body
                && input.before(index) == Some('\n')
                && input.is(index.wrapping_sub(2), '\r')
            {
                index - 2
            } else if index > body {
                index - 1
            } else {
                index
            };
            return Ok(HereString {
                commands,
                source: input.slice(body, end).to_string(),
                end: index + 1,
            });
        }
        if quote == '"' {
            if char == '`' {
                let Some(escape) = escape(input, index, false, skipped) else {
                    return Err(Opaque::UnterminatedEscape);
                };
                index = escape.end + 1;
                line_start = false;
                continue;
            }
            if char == '$' && input.is(index + 1, '{') {
                let Some(end) = braced_variable(input, index, skipped) else {
                    return Err(Opaque::InvalidStructure);
                };
                index = end + 1;
                line_start = false;
                continue;
            }
            if char == '$' && input.is(index + 1, '(') {
                let end = sub_list(
                    input,
                    index + 2,
                    depth,
                    budget,
                    true,
                    skipped,
                    &mut commands,
                )?;
                index = end + 1;
                line_start = false;
                continue;
            }
        }
        line_start = char == '\r' || char == '\n';
        index += 1;
    }
    Err(Opaque::UnterminatedQuote)
}

/// A PowerShell redirect at `index`: `None` when there is none, `Some(None)` when it is malformed.
fn redirect(input: &Src, index: usize) -> Option<Option<String>> {
    static VALID: LazyLock<regex::Regex> =
        LazyLock::new(|| super::regex(r"^(?:(?:[1-6]|\*)?>>?|[2-6*]>&1)$"));
    let mut cursor = index;
    if input.is(cursor, '*') {
        cursor += 1;
    } else {
        while input.at(cursor).is_some_and(|c| c.is_ascii_digit()) {
            cursor += 1;
        }
    }
    if !input.is(cursor, '>') {
        return None;
    }
    cursor += 1;
    if input.is(cursor, '>') {
        cursor += 1;
    }
    if input.is(cursor, '&') {
        cursor += 1;
        while input.at(cursor).is_some_and(|c| c.is_ascii_digit()) {
            cursor += 1;
        }
    }
    let text = input.slice(index, cursor);
    Some(VALID.is_match(text).then(|| text.to_string()))
}
