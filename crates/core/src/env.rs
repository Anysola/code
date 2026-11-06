//! Environment settings: `ANYSOLA_<NAME>` wins; the old `OPENCODE_<NAME>` is still read so setups
//! made for 0.5.x and 1.0.0 keep working.

use std::ffi::OsString;

/// The value of `ANYSOLA_<name>` when set and non-empty, else `OPENCODE_<name>`.
pub fn var_os(name: &str) -> Option<OsString> {
    std::env::var_os(format!("ANYSOLA_{name}"))
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os(format!("OPENCODE_{name}")))
}

/// [`var_os`] as UTF-8; a value that is not is treated as unset.
pub fn var(name: &str) -> Option<String> {
    var_os(name).and_then(|v| v.into_string().ok())
}

/// Set to anything but empty, `0` or `false`.
pub fn flag(name: &str) -> bool {
    var(name).is_some_and(|v| !v.is_empty() && v != "0" && v != "false")
}

/// Set to `1` or `true` (any case, trimmed).
pub fn truthy(name: &str) -> bool {
    var(name).is_some_and(|v| {
        let v = v.trim().to_ascii_lowercase();
        v == "1" || v == "true"
    })
}

/// Both names of a setting, for messages and for scrubbing the environment.
pub fn names(name: &str) -> [String; 2] {
    [format!("ANYSOLA_{name}"), format!("OPENCODE_{name}")]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_new_name_wins_and_the_old_one_is_a_fallback() {
        // Names no other test reads, so setting them cannot race.
        let name = "ENV_COMPAT_TEST";
        let [new, old] = names(name);
        unsafe {
            std::env::remove_var(&new);
            std::env::set_var(&old, "old");
        }
        assert_eq!(var(name).as_deref(), Some("old"));
        unsafe { std::env::set_var(&new, "new") };
        assert_eq!(var(name).as_deref(), Some("new"));
        unsafe { std::env::set_var(&new, "") };
        assert_eq!(var(name).as_deref(), Some("old"));
        unsafe { std::env::set_var(&old, "0") };
        assert!(!flag(name));
        unsafe { std::env::set_var(&new, "true") };
        assert!(flag(name) && truthy(name));
        unsafe {
            std::env::remove_var(&new);
            std::env::remove_var(&old);
        }
        assert_eq!(var(name), None);
    }
}
