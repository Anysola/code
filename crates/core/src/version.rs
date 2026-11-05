//! Version and wire identity strings.

/// The Anysola Code version; release builds set `ANYSOLA_VERSION` at build time.
pub const VERSION: &str = match option_env!("ANYSOLA_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// The OpenCode release this product line descends from; part of the gateway's client contract.
pub const OPENCODE_VERSION: &str = "2.2.5";

/// `anysola --version` output.
pub fn version_line() -> String {
    format!("Anysola Code {VERSION} (OpenCode {OPENCODE_VERSION})")
}

/// User agent of every plain HTTP request.
pub fn user_agent() -> String {
    format!("opencode/latest/{OPENCODE_VERSION}/cli")
}

static CLIENT: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// Marks this process as the ACP server (`anysola acp`), which model calls report.
pub fn set_acp_client() {
    let _ = CLIENT.set("acp");
}

/// `cli`, or `acp` under `anysola acp`.
pub fn client() -> &'static str {
    CLIENT.get().copied().unwrap_or("cli")
}

/// User agent of model calls.
pub fn model_user_agent() -> String {
    format!(
        "opencode/latest/{OPENCODE_VERSION}+anysola.{VERSION}/{}",
        client()
    )
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
        (Some(a), Some(b)) => a.cmp(&b),
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
