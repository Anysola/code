//! ```` ```mermaid ```` blocks as box drawings, after 0.5.1's `@opencode/merman`: flowcharts,
//! sequence diagrams, state diagrams (on the flowchart layout), timelines, gantt charts and git
//! graphs. Other diagrams and charts these layouts cannot draw are errors, and the block stays
//! source.

mod flowchart;
mod gantt;
mod gitgraph;
mod sequence;
mod state;
mod timeline;

pub fn render(source: &str) -> Result<Vec<String>, String> {
    let header = first_line(source).unwrap_or_default();
    let keyword = header.split_whitespace().next().unwrap_or_default();
    match keyword.to_ascii_lowercase().as_str() {
        "timeline" => timeline::render(source),
        "gantt" => gantt::render(source),
        "sequencediagram" => sequence::render(source),
        "gitgraph" | "gitgraph:" => gitgraph::render(source),
        "statediagram" | "statediagram-v2" => state::render(source),
        _ => flowchart::render(source),
    }
}

/// The first line that is not blank or a `%%` comment.
fn first_line(source: &str) -> Option<&str> {
    source
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("%%"))
}

/// The line without a trailing `%%` comment, trimmed.
fn strip_comment(line: &str) -> &str {
    line.find("%%").map_or(line, |at| &line[..at]).trim()
}

/// Quotes removed and HTML entities decoded.
fn unquote(value: &str) -> String {
    let t = value.trim();
    let inner = if t.len() >= 2
        && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')))
    {
        &t[1..t.len() - 1]
    } else {
        t
    };
    decode(inner)
}

/// `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&#39;`, `&#NNN;` and `&#xHH;`.
fn decode(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let entity = tail.find(';').map(|end| (&tail[1..end], end));
        let decoded = entity.and_then(|(name, end)| {
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            }?;
            Some((c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A label's lines: `<br>`, `<br/>` and `<br />` break it.
fn lines(label: &str) -> Vec<String> {
    let mut text = label.to_string();
    for br in ["<br />", "<br/>", "<br>", "<BR/>", "<BR>"] {
        text = text.replace(br, "\n");
    }
    text.split('\n').map(|l| l.trim().to_string()).collect()
}

/// A character grid that grows as it is written to.
struct Grid(Vec<Vec<char>>);

impl Grid {
    fn new(width: usize, height: usize) -> Grid {
        Grid(vec![vec![' '; width]; height])
    }

    fn set(&mut self, x: usize, y: usize, c: char) {
        if let Some(cell) = self.0.get_mut(y).and_then(|row| row.get_mut(x)) {
            *cell = c;
        }
    }

    fn text(&mut self, x: usize, y: usize, text: &str) {
        let mut at = x;
        for c in text.chars() {
            self.set(at, y, c);
            // A wide character's second cell is a placeholder dropped when printing.
            let wide = crate::text::width(c.encode_utf8(&mut [0; 4])).max(1);
            for extra in 1..wide {
                self.set(at + extra, y, '\0');
            }
            at += wide;
        }
    }

    fn rows(self) -> Vec<String> {
        self.0
            .into_iter()
            .map(|row| {
                let text: String = row.into_iter().filter(|c| *c != '\0').collect();
                text.trim_end().to_string()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_entities_and_breaks() {
        assert_eq!(unquote("\"Scale: &#x2265; 10k\""), "Scale: ≥ 10k");
        assert_eq!(decode("a &amp; b &bogus c"), "a & b &bogus c");
        assert_eq!(lines("a<br/>b"), ["a", "b"]);
    }

    #[test]
    fn unsupported_diagrams_fall_back() {
        assert!(render("erDiagram\nA ||--o{ B : has").is_err());
        assert!(render("stateDiagram-v2\n[*] --> A").is_ok());
    }
}
