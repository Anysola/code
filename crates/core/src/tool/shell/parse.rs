//! What a shell call asks permission for (0.5.1's `shell/parse.ts`): the scanned commands with
//! their "always" save patterns, and the directories static `cd`-like commands change to.

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use super::scan::{self, Dialect, Opaque};
use super::select;

/// A permission resource and the pattern "always" saves for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    pub resource: String,
    pub save: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    pub commands: Vec<Resource>,
    /// Static `cd` targets, as written after `~`/variable expansion (relative ones stay relative).
    pub directories: Vec<String>,
}

/// What directory expansion may read: the home directory and the environment.
pub struct Host<'a> {
    pub home: &'a str,
    pub env: &'a dyn Fn(&str) -> Option<String>,
}

const CWD: &[&str] = &[
    "cd",
    "chdir",
    "popd",
    "pushd",
    "push-location",
    "set-location",
];

static ARITY: LazyLock<HashMap<&'static str, usize>> = LazyLock::new(|| {
    [
        ("cat", 1),
        ("cd", 1),
        ("chmod", 1),
        ("chown", 1),
        ("cp", 1),
        ("echo", 1),
        ("env", 1),
        ("export", 1),
        ("grep", 1),
        ("kill", 1),
        ("killall", 1),
        ("ln", 1),
        ("ls", 1),
        ("mkdir", 1),
        ("mv", 1),
        ("ps", 1),
        ("pwd", 1),
        ("rm", 1),
        ("rmdir", 1),
        ("sleep", 1),
        ("source", 1),
        ("tail", 1),
        ("touch", 1),
        ("unset", 1),
        ("which", 1),
        ("aws", 3),
        ("az", 3),
        ("bazel", 2),
        ("brew", 2),
        ("bun", 2),
        ("bun run", 3),
        ("bun x", 3),
        ("cargo", 2),
        ("cargo add", 3),
        ("cargo run", 3),
        ("cdk", 2),
        ("cf", 2),
        ("cmake", 2),
        ("composer", 2),
        ("consul", 2),
        ("consul kv", 3),
        ("crictl", 2),
        ("deno", 2),
        ("deno task", 3),
        ("doctl", 3),
        ("docker", 2),
        ("docker builder", 3),
        ("docker compose", 3),
        ("docker container", 3),
        ("docker image", 3),
        ("docker network", 3),
        ("docker volume", 3),
        ("eksctl", 2),
        ("eksctl create", 3),
        ("firebase", 2),
        ("flyctl", 2),
        ("gcloud", 3),
        ("gh", 3),
        ("git", 2),
        ("git config", 3),
        ("git remote", 3),
        ("git stash", 3),
        ("go", 2),
        ("gradle", 2),
        ("helm", 2),
        ("heroku", 2),
        ("hugo", 2),
        ("ip", 2),
        ("ip addr", 3),
        ("ip link", 3),
        ("ip netns", 3),
        ("ip route", 3),
        ("kind", 2),
        ("kind create", 3),
        ("kubectl", 2),
        ("kubectl kustomize", 3),
        ("kubectl rollout", 3),
        ("kustomize", 2),
        ("make", 2),
        ("mc", 2),
        ("mc admin", 3),
        ("minikube", 2),
        ("mongosh", 2),
        ("mysql", 2),
        ("mvn", 2),
        ("ng", 2),
        ("npm", 2),
        ("npm exec", 3),
        ("npm init", 3),
        ("npm run", 3),
        ("npm view", 3),
        ("npx", 2),
        ("nvm", 2),
        ("nx", 2),
        ("openssl", 2),
        ("openssl req", 3),
        ("openssl x509", 3),
        ("pip", 2),
        ("pipenv", 2),
        ("pnpm", 2),
        ("pnpm dlx", 3),
        ("pnpm exec", 3),
        ("pnpm run", 3),
        ("poetry", 2),
        ("podman", 2),
        ("podman container", 3),
        ("podman image", 3),
        ("psql", 2),
        ("pulumi", 2),
        ("pulumi stack", 3),
        ("python", 2),
        ("pyenv", 2),
        ("rake", 2),
        ("rbenv", 2),
        ("redis-cli", 2),
        ("rustup", 2),
        ("serverless", 2),
        ("sfdx", 3),
        ("skaffold", 2),
        ("sls", 2),
        ("sst", 2),
        ("swift", 2),
        ("systemctl", 2),
        ("terraform", 2),
        ("terraform workspace", 3),
        ("tmux", 2),
        ("turbo", 2),
        ("ufw", 2),
        ("vault", 2),
        ("vault auth", 3),
        ("vault kv", 3),
        ("vercel", 2),
        ("volta", 2),
        ("wp", 2),
        ("yarn", 2),
        ("yarn dlx", 3),
        ("yarn run", 3),
    ]
    .into_iter()
    .collect()
});
const PREFIX_LENGTH: usize = 3;

fn regex(pattern: &str) -> regex::Regex {
    regex::Regex::new(pattern).expect("parse regex is valid")
}

static PATH_FLAG: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex(
        r"(?i-u)^-(?:p(?:ath|at|a)?|lp|l(?:i(?:t(?:e(?:r(?:a(?:l(?:p(?:a(?:t(?:h)?)?)?)?)?)?)?)?)?)?|psp(?:a(?:t(?:h)?)?)?):?$",
    )
});
static LITERAL_FLAG: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex(
        r"(?i-u)^-(?:lp|l(?:i(?:t(?:e(?:r(?:a(?:l(?:p(?:a(?:t(?:h)?)?)?)?)?)?)?)?)?)?|psp(?:a(?:t(?:h)?)?)?):?$",
    )
});
static STACK_FLAG: LazyLock<regex::Regex> =
    LazyLock::new(|| regex(r"(?i-u)^-(?:stack(?:name|nam|na|n)?|st):?$"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PartKind {
    Word,
    Parameter,
}

#[derive(Debug, Clone)]
struct Part {
    kind: PartKind,
    text: String,
    raw: String,
    unquoted_tilde: bool,
}

impl Part {
    fn word(text: String, raw: String) -> Part {
        Part {
            kind: PartKind::Word,
            text,
            raw,
            unquoted_tilde: false,
        }
    }

    fn parameter(text: String, raw: String) -> Part {
        Part {
            kind: PartKind::Parameter,
            text,
            raw,
            unquoted_tilde: false,
        }
    }
}

/// `"Portable shell scanner cannot analyze command: <reason>"` on failure.
pub fn error_text(reason: Opaque) -> String {
    format!("Portable shell scanner cannot analyze command: {reason}")
}

/// Scans `command` as `shell` would read it, `cwd` being its working directory.
pub fn scan(command: &str, shell: &str, cwd: &str, host: &Host) -> Result<Parsed, Opaque> {
    let name = select::name(shell);
    let powershell = name == "pwsh" || name == "powershell";
    let scanned = if powershell {
        scan::scan_powershell(command)?
    } else {
        let dialect = match name.as_str() {
            "bash" => Dialect::Bash,
            "zsh" => Dialect::Zsh,
            _ => Dialect::Posix,
        };
        scan::scan(command, dialect)?
    };
    static FOREACH: LazyLock<regex::Regex> = LazyLock::new(|| regex(r"(?i-u)^foreach(?:-|$)"));
    let mut output = Parsed::default();
    for item in scanned {
        // Declarations are skipped as standalone permission resources, while nested substitutions remain.
        if item.declaration {
            continue;
        }
        let resource: Vec<char> = item.resource.chars().collect();
        if let Some(end) = item.redirect_prefix_end {
            let head: String = resource[..end.min(resource.len())].iter().collect();
            let separator = match resource.get(end) {
                Some(&c @ ('\t' | '\n')) => c,
                _ => ' ',
            };
            output.commands.push(Resource {
                resource: item.resource.clone(),
                save: format!("{head}{separator}*"),
            });
            continue;
        }
        let words = match item.redirect_word_count {
            Some(count) => &item.raw_words[..count.min(item.raw_words.len())],
            None => &item.raw_words[..],
        };
        // Bare statement-head foreach blocks act as PowerShell control flow rather than cmdlet invocations.
        if powershell
            && item.statement_head
            && FOREACH.is_match(words.first().map_or("", String::as_str))
            && words.len() > 1
            && words[1..].iter().all(|w| w.starts_with('{'))
        {
            continue;
        }
        let parts: Vec<Part> = if powershell {
            words
                .iter()
                .enumerate()
                .flat_map(|(index, raw)| powershell_parts(raw, item.words.get(index)))
                .collect()
        } else {
            item.raw_words
                .iter()
                .enumerate()
                .map(|(index, raw)| {
                    let word = item.words.get(index).unwrap_or(raw);
                    Part {
                        kind: PartKind::Word,
                        text: if static_bash_word(raw) {
                            word.replace('$', "\0")
                        } else {
                            "$".into()
                        },
                        raw: raw.clone(),
                        unquoted_tilde: raw.starts_with('~'),
                    }
                })
                .collect()
        };
        let heads = if powershell {
            words
        } else {
            &item.raw_words[..]
        };
        if let Some((directory_command, whole)) =
            unwrap_directory_command(heads, &parts, powershell)
        {
            let (directories, complete) =
                directory_args(directory_command, powershell, cwd, shell, host);
            output.directories.extend(directories);
            if whole && complete {
                continue;
            }
        }
        let selected = prefix(&words[..words.len().min(PREFIX_LENGTH)]);
        let conventional = format!("{} *", selected.join(" "));
        let end = item
            .word_ends
            .as_ref()
            .and_then(|ends| selected.len().checked_sub(1).and_then(|i| ends.get(i)))
            .copied();
        // Keep existing grants stable unless normalized spacing loses the original source boundary.
        let save = match end {
            Some(end)
                if powershell && !crate::permission::matches(&item.resource, &conventional) =>
            {
                let before = |at: usize| {
                    resource[..at.min(resource.len())]
                        .iter()
                        .collect::<String>()
                };
                if resource.get(end) == Some(&'(') {
                    format!("{}(*", before(end))
                } else {
                    let boundary = item
                        .word_ends
                        .iter()
                        .flatten()
                        .copied()
                        .find(|&v| v >= end && resource.get(v).is_none_or(|&c| scan::js_space(c)))
                        .unwrap_or(end);
                    static SEPARATOR: LazyLock<regex::Regex> = LazyLock::new(|| {
                        regex(&format!(
                            r"^[{s}]+(?:`(?:\r\n|\r|\n)[{s}]*)?",
                            s = scan::JS_SPACE_SET
                        ))
                    });
                    let rest: String = resource[boundary.min(resource.len())..].iter().collect();
                    let separator = SEPARATOR.find(&rest).map_or(" ", |m| m.as_str());
                    format!("{}{separator}*", before(boundary))
                }
            }
            _ => conventional,
        };
        output.commands.push(Resource {
            resource: item.resource,
            save,
        });
    }
    Ok(output)
}

fn powershell_parts(raw: &str, word: Option<&String>) -> Vec<Part> {
    static PARAMETER: LazyLock<regex::Regex> =
        LazyLock::new(|| regex(r"^-[A-Za-z_?][A-Za-z0-9_?-]*(?::|$)"));
    let text = if !raw.contains("`$") && raw.contains(['\'', '"', '`']) {
        word.cloned().unwrap_or_else(|| raw.to_string())
    } else {
        raw.replace("`$", "\0")
    };
    if !PARAMETER.is_match(raw) {
        return vec![Part::word(text, raw.to_string())];
    }
    let Some(colon) = text.find(':') else {
        return vec![Part::parameter(text, raw.to_string())];
    };
    let raw_colon = raw.find(':');
    if colon + 1 < text.len() {
        // `raw.slice(0, -1)` when the raw word has no colon.
        let raw_head = match raw_colon {
            Some(at) => raw[..at].to_string(),
            None => raw
                .chars()
                .take(raw.chars().count().saturating_sub(1))
                .collect(),
        };
        let raw_tail = match raw_colon {
            Some(at) => raw[at + 1..].to_string(),
            None => raw.to_string(),
        };
        vec![
            Part::parameter(text[..colon].to_string(), raw_head),
            Part::word(text[colon + 1..].to_string(), raw_tail),
        ]
    } else {
        vec![Part::parameter(text[..colon].to_string(), raw.to_string())]
    }
}

/// The `cd`-like command among `parts` (after `builtin`/`command` wrappers), and whether it is the
/// whole command.
fn unwrap_directory_command<'p>(
    raw_words: &[String],
    parts: &'p [Part],
    powershell: bool,
) -> Option<(&'p [Part], bool)> {
    let head = raw_words.first().map(|w| {
        if powershell {
            w.to_lowercase()
        } else {
            w.clone()
        }
    });
    if CWD.contains(&head.as_deref().unwrap_or("")) {
        return Some((parts, true));
    }
    if powershell {
        return None;
    }
    static COMMAND_V: LazyLock<regex::Regex> = LazyLock::new(|| regex(r"^-[p]*[vV]"));
    let word = |i: usize| raw_words.get(i).map(String::as_str);
    let mut index = 0;
    while let Some(wrapper @ ("builtin" | "command")) = word(index) {
        index += 1;
        while let Some(flag) = word(index).filter(|w| w.starts_with('-')) {
            index += 1;
            if flag == "--" {
                break;
            }
            if wrapper == "command" && COMMAND_V.is_match(flag) {
                return None;
            }
        }
    }
    (index > 0 && CWD.contains(&word(index).unwrap_or("")))
        .then(|| (parts.get(index..).unwrap_or_default(), false))
}

/// Whether a raw bash word expands to fixed text: no expansions, no globs in dot segments, no brace
/// lists.
fn static_bash_word(raw: &str) -> bool {
    #[derive(PartialEq)]
    enum Quote {
        Single,
        Double,
        Ansi,
    }
    struct Segments {
        started: bool,
        leading_dot: bool,
        glob: bool,
        ansi_escape: bool,
        unquoted_glob: bool,
    }
    impl Segments {
        fn record(&mut self, c: char, unquoted_glob: bool) -> bool {
            if unquoted_glob {
                self.unquoted_glob = true;
            }
            if self.ansi_escape && self.unquoted_glob {
                return false;
            }
            if c == '/' {
                self.started = false;
                self.leading_dot = false;
                self.glob = false;
                return true;
            }
            if !self.started {
                self.started = true;
                self.leading_dot = c == '.';
            }
            if unquoted_glob {
                self.glob = true;
            }
            !(self.leading_dot && self.glob)
        }
    }
    let expansion_start = |next: Option<char>| {
        next.is_some_and(|n| n.is_ascii_alphanumeric() || "_@*#?$!({[-".contains(n))
    };
    let chars: Vec<char> = raw.chars().collect();
    let continuation = |i: usize| chars.get(i) == Some(&'\\') && chars.get(i + 1) == Some(&'\n');
    let mut quote: Option<Quote> = None;
    let mut segments = Segments {
        started: false,
        leading_dot: false,
        glob: false,
        ansi_escape: false,
        unquoted_glob: false,
    };
    let mut braces: Vec<bool> = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        let next = chars.get(index + 1).copied();
        match quote {
            Some(Quote::Single) => {
                if c == '\'' {
                    quote = None;
                } else if !segments.record(c, false) {
                    return false;
                }
            }
            Some(Quote::Ansi) => {
                if c == '\\' {
                    segments.ansi_escape = true;
                    if segments.unquoted_glob {
                        return false;
                    }
                    index += 1;
                } else if c == '\'' {
                    quote = None;
                } else if !segments.record(c, false) {
                    return false;
                }
            }
            Some(Quote::Double) => {
                if c == '\\' && next.is_some_and(|n| "$`\"\\\n".contains(n)) {
                    index += 1;
                    if let Some(n) = next
                        && n != '\n'
                        && !segments.record(n, false)
                    {
                        return false;
                    }
                } else if c == '"' {
                    quote = None;
                } else if c == '`'
                    || (c == '$' && (expansion_start(next) || continuation(index + 1)))
                    || !segments.record(c, false)
                {
                    return false;
                }
            }
            None => {
                if c == '\\' {
                    index += 1;
                    if next.is_some_and(|n| n != '\n')
                        && !segments.record(next.unwrap_or_default(), false)
                    {
                        return false;
                    }
                } else if c == '\'' {
                    quote = Some(Quote::Single);
                } else if c == '"' {
                    quote = Some(Quote::Double);
                } else if c == '$' && next == Some('\'') {
                    quote = Some(Quote::Ansi);
                    index += 1;
                } else if c == '$' && next == Some('"') {
                    quote = Some(Quote::Double);
                    index += 1;
                } else if c == '`'
                    || ("<>=".contains(c) && next == Some('('))
                    || (c == '$' && (expansion_start(next) || continuation(index + 1)))
                {
                    return false;
                } else if c == '{' {
                    braces.push(false);
                    if !segments.record(c, false) {
                        return false;
                    }
                } else {
                    if !braces.is_empty() && (c == ',' || (c == '.' && next == Some('.'))) {
                        *braces.last_mut().expect("non-empty") = true;
                    }
                    if c == '}' && !braces.is_empty() {
                        if braces.pop() == Some(true) || !segments.record(c, false) {
                            return false;
                        }
                    } else if !segments.record(c, matches!(c, '*' | '?' | '[')) {
                        return false;
                    }
                }
            }
        }
        index += 1;
    }
    quote.is_none()
}

/// Whether a raw PowerShell word globs a segment that starts with a dot.
fn powershell_dot_wildcard(raw: &str) -> bool {
    struct Segments {
        started: bool,
        leading_dot: bool,
        glob: bool,
        quote_escape: bool,
        unquoted_glob: bool,
    }
    impl Segments {
        fn record(&mut self, c: char, unquoted_glob: bool) -> bool {
            if unquoted_glob {
                self.unquoted_glob = true;
            }
            if self.quote_escape && self.unquoted_glob {
                return true;
            }
            if c == '/' || c == '\\' {
                self.started = false;
                self.leading_dot = false;
                self.glob = false;
                return false;
            }
            if !self.started {
                self.started = true;
                self.leading_dot = c == '.';
            }
            if unquoted_glob {
                self.glob = true;
            }
            self.leading_dot && self.glob
        }
    }
    let chars: Vec<char> = raw.chars().collect();
    let mut quote: Option<char> = None;
    let mut segments = Segments {
        started: false,
        leading_dot: false,
        glob: false,
        quote_escape: false,
        unquoted_glob: false,
    };
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        let next = chars.get(index + 1).copied();
        match quote {
            Some('\'') => {
                if c == '\'' && next == Some('\'') {
                    index += 1;
                    if segments.record('\'', false) {
                        return true;
                    }
                } else if c == '\'' {
                    quote = None;
                } else if segments.record(c, false) {
                    return true;
                }
            }
            Some(_) => {
                if c == '`' {
                    segments.quote_escape = true;
                    if segments.unquoted_glob {
                        return true;
                    }
                    index += 1;
                } else if c == '"' && next == Some('"') {
                    index += 1;
                    if segments.record('"', false) {
                        return true;
                    }
                } else if c == '"' {
                    quote = None;
                } else if segments.record(c, false) {
                    return true;
                }
            }
            None => {
                if c == '`' {
                    index += 1;
                    // An escape at the very end records an empty character, which starts no segment.
                    if let Some(n) = next
                        && segments.record(n, false)
                    {
                        return true;
                    }
                } else if c == '\'' || c == '"' {
                    quote = Some(c);
                } else if segments.record(c, matches!(c, '*' | '?' | '[')) {
                    return true;
                }
            }
        }
        index += 1;
    }
    false
}

fn directory_args(
    command: &[Part],
    powershell: bool,
    cwd: &str,
    shell: &str,
    host: &Host,
) -> (Vec<String>, bool) {
    let head = command
        .first()
        .map(|p| p.text.to_lowercase())
        .unwrap_or_default();
    if head == "popd" || head == "pop-location" {
        return (Vec::new(), false);
    }
    if !powershell {
        let mut end_of_options = false;
        let operands: Vec<&Part> = command
            .iter()
            .skip(1)
            .filter(|part| {
                if end_of_options {
                    return true;
                }
                if part.text == "--" {
                    end_of_options = true;
                    return false;
                }
                part.text == "-" || !part.text.starts_with('-')
            })
            .collect();
        if operands.is_empty() {
            return (Vec::new(), false);
        }
        let directories: Vec<String> = operands
            .iter()
            .filter(|part| part.text != "-")
            .filter_map(|part| {
                directory_argument(&part.text, false, cwd, shell, part.unquoted_tilde, host)
            })
            .collect();
        let complete = directories.len() == operands.len();
        return (directories, complete);
    }
    let mut directories = Vec::new();
    let mut path_flag = false;
    let mut literal_flag = false;
    let mut end_of_parameters = false;
    let mut saw_operand = false;
    let mut complete = true;
    for part in &command[1.min(command.len())..] {
        if !end_of_parameters && part.text == "--" {
            end_of_parameters = true;
            continue;
        }
        if path_flag {
            saw_operand = true;
            let value = if !literal_flag
                && (powershell_dot_wildcard(&part.raw) || part.text == "-" || part.text == "+")
            {
                None
            } else {
                directory_argument(&part.text, true, cwd, shell, false, host)
            };
            match value {
                Some(value) if !value.is_empty() => directories.push(value),
                _ => complete = false,
            }
            path_flag = false;
            literal_flag = false;
            continue;
        }
        if !end_of_parameters && part.kind == PartKind::Parameter {
            if STACK_FLAG.is_match(&part.text) {
                return (directories, false);
            }
            path_flag = PATH_FLAG.is_match(&part.text);
            literal_flag = LITERAL_FLAG.is_match(&part.text);
            continue;
        }
        saw_operand = true;
        let value = if powershell_dot_wildcard(&part.raw)
            || (!end_of_parameters && (part.raw == "-" || part.raw == "+"))
        {
            None
        } else {
            directory_argument(&part.text, true, cwd, shell, false, host)
        };
        match value {
            Some(value) if !value.is_empty() => directories.push(value),
            _ => complete = false,
        }
    }
    (directories, saw_operand && complete)
}

fn directory_argument(
    value: &str,
    powershell: bool,
    cwd: &str,
    shell: &str,
    unquoted_tilde: bool,
    host: &Host,
) -> Option<String> {
    if !powershell {
        return expand_known_directory(value, false, cwd, unquoted_tilde, host);
    }
    let first = value.chars().next();
    let text = match first {
        Some(q @ ('"' | '\'')) if value.chars().count() >= 2 && value.ends_with(q) => {
            &value[1..value.len() - 1]
        }
        // A lone quote is both the first and the last character.
        Some('"' | '\'') if value.chars().count() == 1 => "",
        _ => value,
    };
    // PowerShell exposes environment variables through $env:NAME and provides these automatic
    // directory variables. Expand only values we can determine without executing code.
    static BRACED_ENV: LazyLock<regex::Regex> =
        LazyLock::new(|| regex(r"\$\{(?i-u:env):([^}]+)\}"));
    static ENV: LazyLock<regex::Regex> =
        LazyLock::new(|| regex(r"\$(?i-u:env):([A-Za-z_][A-Za-z0-9_]*)"));
    static AUTOMATIC: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
        fancy_regex::Regex::new(r"(?i)\$(?:\{(HOME|PWD|PSHOME)\}|(HOME|PWD|PSHOME)(?=$|[\\/]))")
            .expect("parse regex is valid")
    });
    let lookup = |name: &str| environment(name, host).unwrap_or_default();
    let text = BRACED_ENV.replace_all(text, |c: &regex::Captures| lookup(&c[1]));
    let text = ENV.replace_all(&text, |c: &regex::Captures| lookup(&c[1]));
    let text = AUTOMATIC.replace_all(&text, |c: &fancy_regex::Captures| {
        let key = c
            .get(1)
            .or_else(|| c.get(2))
            .map_or("", |m| m.as_str())
            .to_uppercase();
        match key.as_str() {
            "HOME" => host.home.to_string(),
            "PWD" => cwd.to_string(),
            _ => dirname(shell),
        }
    });
    expand_known_directory(&text, true, cwd, false, host)
}

fn environment(key: &str, host: &Host) -> Option<String> {
    (host.env)(key)
}

fn expand_known_directory(
    value: &str,
    powershell: bool,
    cwd: &str,
    unquoted_tilde: bool,
    host: &Host,
) -> Option<String> {
    // Unknown shell expressions cannot be resolved safely during permission analysis.
    if value.contains(['$', '`']) || (powershell && value.starts_with('(')) {
        return None;
    }
    let resolved = value.replace('\0', "$");
    if resolved == "~" {
        return Some(host.home.to_string());
    }
    if let Some(rest) = resolved
        .strip_prefix("~/")
        .or_else(|| resolved.strip_prefix("~\\").filter(|_| cfg!(windows)))
    {
        return Some(join(host.home, rest));
    }
    if unquoted_tilde && !cfg!(windows) {
        if resolved == "~+" {
            return Some(cwd.to_string());
        }
        if let Some(rest) = resolved.strip_prefix("~+/") {
            return Some(join(cwd, rest));
        }
        static STACK: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
            fancy_regex::Regex::new(r"^~(?:[-+](?:[0-9]+)?|[0-9]+)(?=$|/)")
                .expect("parse regex is valid")
        });
        static USER: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
            fancy_regex::Regex::new(r"^~([A-Za-z_][A-Za-z0-9._-]*)(?=$|/)")
                .expect("parse regex is valid")
        });
        if STACK.is_match(&resolved).unwrap_or(false) {
            return None;
        }
        if let Ok(Some(found)) = USER.captures(&resolved) {
            let user = &found[1];
            let home = user_home(user)?;
            return Some(join(&home, &resolved[user.len() + 1..]));
        }
    }
    Some(resolved)
}

/// A user's home directory from the user database (and `/Users/<user>` on macOS).
fn user_home(user: &str) -> Option<String> {
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let found = passwd
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .find_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            (fields.first() == Some(&user))
                .then(|| fields.get(5).copied())
                .flatten()
                .filter(|home| !home.is_empty())
                .map(str::to_string)
        });
    if found.is_some() {
        return found;
    }
    let candidate = Path::new("/Users").join(user);
    (cfg!(target_os = "macos") && candidate.is_dir()).then(|| candidate.display().to_string())
}

/// `path.dirname`.
fn dirname(file: &str) -> String {
    let separators: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    let trimmed = file.trim_end_matches(separators);
    match trimmed.rfind(separators) {
        Some(0) => file[..1].to_string(),
        Some(at) => trimmed[..at].trim_end_matches(separators).to_string(),
        None if file.starts_with(separators) => file[..1].to_string(),
        None => ".".into(),
    }
}

/// `path.join(base, rest)`: joined, then `.`/`..` and repeated separators resolved, a trailing
/// separator kept.
fn join(base: &str, rest: &str) -> String {
    if cfg!(windows) {
        return crate::paths::normalize(&Path::new(base).join(rest))
            .display()
            .to_string();
    }
    let joined = if rest.is_empty() {
        base.to_string()
    } else if base.is_empty() {
        rest.to_string()
    } else {
        format!("{base}/{rest}")
    };
    let absolute = joined.starts_with('/');
    let trailing = joined.ends_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|p| *p != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let mut out = parts.join("/");
    if absolute {
        out.insert(0, '/');
    }
    if out.is_empty() {
        out.push('.');
    }
    if trailing && !out.ends_with('/') {
        out.push('/');
    }
    out
}

/// The command prefix "always" saves: the longest ARITY key among the first words, cut to its
/// arity; otherwise the first word.
fn prefix(tokens: &[String]) -> Vec<String> {
    for length in (1..=tokens.len()).rev() {
        if let Some(&arity) = ARITY.get(tokens[..length].join(" ").as_str()) {
            return tokens[..arity.min(tokens.len())].to_vec();
        }
    }
    tokens.iter().take(1).cloned().collect()
}
