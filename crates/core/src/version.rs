//! Version and wire identity strings.

/// The Anysola Code version; release builds set `ANYSOLA_VERSION` at build time.
pub const VERSION: &str = match option_env!("ANYSOLA_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// The base version of the stored session format (`<base>+anysola.<ver>`), kept so 0.5.x and 1.0.0
/// databases read the same; not sent to any server.
pub const OPENCODE_VERSION: &str = "2.2.5";

/// User agent of every plain HTTP request.
pub fn user_agent() -> String {
    format!("anysola-code/{VERSION}")
}

static CLIENT: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// Marks this process as the desktop app, which model calls report. The desktop app has no
/// console, so the processes it starts open no console window either.
pub fn set_desktop_client() {
    let _ = CLIENT.set("desktop");
    crate::console::set_no_console();
}

/// `desktop` in the desktop app; `cli` elsewhere (tests).
pub fn client() -> &'static str {
    CLIENT.get().copied().unwrap_or("cli")
}

/// User agent of model calls: the plain one plus `desktop` (`cli` in tests).
pub fn model_user_agent() -> String {
    format!("anysola-code/{VERSION}/{}", client())
}

/// Stored on sessions as the writer's version.
pub fn session_version() -> String {
    format!("{OPENCODE_VERSION}+anysola.{VERSION}")
}

/// Development builds skip the minimum-version gate.
pub fn is_dev_build(version: &str) -> bool {
    let numeric = version.split('.').take(3).count() == 3
        && version
            .split(['.', '-', '+'])
            .take(3)
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    !numeric || version.starts_with("0.0.0") || version.ends_with("-dev")
}

/// Semver order: numeric core first, then a prerelease sorts before its release.
pub fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |v: &str| {
        let v = v.strip_prefix('v').unwrap_or(v);
        // Build metadata does not take part in the order.
        let v = v.split_once('+').map_or(v, |(v, _)| v);
        let (core, pre) = v
            .split_once('-')
            .map_or((v, None), |(c, p)| (c, Some(p.to_string())));
        let nums: Vec<u64> = core.split('.').map(|n| n.parse().unwrap_or(0)).collect();
        (nums, pre)
    };
    let (an, ap) = parse(a);
    let (bn, bp) = parse(b);
    for i in 0..3 {
        let order = an
            .get(i)
            .copied()
            .unwrap_or(0)
            .cmp(&bn.get(i).copied().unwrap_or(0));
        if order.is_ne() {
            return order;
        }
    }
    match (ap, bp) {
        (None, None) => std::cmp::Ordering::Equal,
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(a), Some(b)) => prerelease(&a, &b),
    }
}

/// Semver prerelease order: dot-separated identifiers left to right, numeric ones by value and
/// below alphanumeric ones, a shorter list first when it is a prefix (`rc.9` < `rc.10`).
fn prerelease(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    // Digits by value without parsing, so any length works: fewer digits first, then the text.
    fn numeric(s: &str) -> Option<&str> {
        (!s.is_empty() && s.bytes().all(|c| c.is_ascii_digit())).then(|| s.trim_start_matches('0'))
    }
    let mut left = a.split('.');
    let mut right = b.split('.');
    loop {
        let order = match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => match (numeric(x), numeric(y)) {
                (Some(x), Some(y)) => x.len().cmp(&y.len()).then_with(|| x.cmp(y)),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => x.cmp(y),
            },
        };
        if order.is_ne() {
            return order;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::*;

    #[test]
    fn ordering() {
        assert_eq!(compare("0.5.1", "0.2.0"), Greater);
        assert_eq!(compare("v1.0.0", "1.0.0"), Equal);
        assert_eq!(compare("1.0.0-rc.1", "1.0.0"), Less);
        assert_eq!(compare("1.0.0-a", "1.0.0-b"), Less);
        assert_eq!(compare("0.10.0", "0.9.9"), Greater);
        assert_eq!(compare("1.1.0-rc.10", "1.1.0-rc.9"), Greater);
        assert_eq!(compare("1.0.0-rc", "1.0.0-rc.1"), Less);
        assert_eq!(compare("1.0.0-1", "1.0.0-alpha"), Less);
        assert_eq!(compare("1.0.0-alpha.beta", "1.0.0-alpha.1"), Greater);
        assert_eq!(compare("1.0.5+build.7", "1.0.5"), Equal);
        assert_eq!(compare("1.0.0-rc.1+x", "1.0.0-rc.1"), Equal);
    }

    #[test]
    fn dev_builds() {
        assert!(is_dev_build("1.0.0-dev"));
        assert!(is_dev_build("0.0.0-local"));
        assert!(is_dev_build("local"));
        assert!(!is_dev_build("1.0.0"));
        assert!(!is_dev_build("1.0.0-rc.1"));
    }
}
