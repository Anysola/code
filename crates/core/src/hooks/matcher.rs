//! Which tool names (or session sources) a handler group applies to: everything, a list of exact
//! names, or an unanchored regular expression.

#[derive(Debug, Clone)]
pub enum Matcher {
    All,
    /// Exact, case-sensitive names.
    Names(Vec<String>),
    Regex(fancy_regex::Regex),
    /// An invalid regex: matches nothing.
    Never,
}

impl Matcher {
    /// The matcher for a `matcher` value, with a warning when it cannot be used.
    pub fn parse(matcher: Option<&str>) -> (Matcher, Option<String>) {
        let Some(text) = matcher
            .map(str::trim)
            .filter(|m| !m.is_empty() && *m != "*")
        else {
            return (Matcher::All, None);
        };
        let plain = text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ' ' | ',' | '|'));
        if plain {
            let names: Vec<String> = text
                .split(['|', ','])
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string)
                .collect();
            return (Matcher::Names(names), None);
        }
        match fancy_regex::Regex::new(text) {
            Ok(regex) => (Matcher::Regex(regex), None),
            Err(_) => (
                Matcher::Never,
                Some(format!(
                    "Ignoring hooks with matcher \"{text}\": it is not a valid regular expression."
                )),
            ),
        }
    }

    /// Whether any of `names` (a tool's two names, or a session source) matches.
    pub fn matches(&self, names: &[&str]) -> bool {
        match self {
            Matcher::All => true,
            Matcher::Names(list) => names.iter().any(|n| list.iter().any(|l| l == n)),
            Matcher::Regex(regex) => names.iter().any(|n| regex.is_match(n).unwrap_or(false)),
            Matcher::Never => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(matcher: Option<&str>, names: &[&str]) -> bool {
        Matcher::parse(matcher).0.matches(names)
    }

    #[test]
    fn empty_star_and_absent_match_everything() {
        for matcher in [None, Some(""), Some("*"), Some("  ")] {
            assert!(matches(matcher, &["shell", "Bash"]));
        }
    }

    #[test]
    fn plain_lists_are_exact_names() {
        assert!(matches(Some("Bash"), &["shell", "Bash"]));
        assert!(matches(Some("shell"), &["shell", "Bash"]));
        assert!(matches(Some("Edit|Write"), &["write", "Write"]));
        assert!(matches(Some("Edit, Write"), &["edit", "Edit"]));
        assert!(!matches(Some("bash"), &["shell", "Bash"]), "case-sensitive");
        assert!(!matches(Some("Bas"), &["shell", "Bash"]), "not a prefix");
        assert!(matches(
            Some("mcp__fake__touch"),
            &["fake_touch", "mcp__fake__touch"]
        ));
    }

    #[test]
    fn anything_else_is_an_unanchored_regex() {
        assert!(matches(
            Some("mcp__.*"),
            &["fake_touch", "mcp__fake__touch"]
        ));
        assert!(matches(Some("Ba.h"), &["shell", "Bash"]));
        assert!(matches(Some("^Wr"), &["write", "Write"]));
        assert!(!matches(Some("^rite"), &["write", "Write"]));
    }

    #[test]
    fn an_invalid_regex_matches_nothing_and_warns() {
        let (matcher, warning) = Matcher::parse(Some("Bash("));
        assert!(!matcher.matches(&["shell", "Bash"]));
        assert!(warning.unwrap().contains("Bash("));
    }
}
