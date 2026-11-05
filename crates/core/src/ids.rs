//! Identifiers: 12 hex characters of `(epoch ms * 0x1000 + counter)` followed by 14 random base62
//! characters, with a type prefix. Session ids sort newest first, so their time part is inverted.

use std::sync::Mutex;

use rand::Rng;

const CHARS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

static CLOCK: Mutex<(u64, u64)> = Mutex::new((0, 0));

fn create(descending: bool) -> String {
    let now = crate::time::now_ms();
    let counter = {
        let mut clock = CLOCK.lock().unwrap_or_else(|e| e.into_inner());
        if clock.0 != now {
            *clock = (now, 0);
        }
        clock.1 += 1;
        clock.1
    };
    let current = now.wrapping_mul(0x1000).wrapping_add(counter);
    let value = if descending { !current } else { current };
    let mut out = String::with_capacity(26);
    for index in 0..6 {
        let byte = (value >> (40 - 8 * index)) & 0xff;
        out.push_str(&format!("{byte:02x}"));
    }
    let mut rng = rand::thread_rng();
    for _ in 0..14 {
        out.push(CHARS[rng.gen_range(0..=255u8) as usize % 62] as char);
    }
    out
}

pub fn schedule() -> String {
    format!("sch_{}", create(false))
}

pub fn session() -> String {
    format!("ses_{}", create(true))
}

pub fn message() -> String {
    format!("msg_{}", create(false))
}

pub fn part() -> String {
    format!("prt_{}", create(false))
}

pub fn event() -> String {
    format!("evt_{}", create(false))
}

pub fn shell() -> String {
    format!("sh_{}", create(false))
}

pub fn tool_output() -> String {
    format!("tool_{}", create(false))
}

pub fn permission() -> String {
    format!("per_{}", create(false))
}

pub fn form() -> String {
    format!("frm_{}", create(false))
}

pub fn credential() -> String {
    format!("cred_{}", create(false))
}

pub fn saved_permission() -> String {
    format!("psv_{}", create(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_and_order() {
        let a = message();
        let b = message();
        assert_eq!(a.len(), 4 + 26);
        assert!(a < b, "ascending ids sort by creation");
        let s1 = session();
        let s2 = session();
        assert!(s1 > s2, "session ids sort newest first");
        assert!(s1.starts_with("ses_"));
    }
}
