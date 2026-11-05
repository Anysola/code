//! Account figures as `anysola usage`/`status` and the TUI print them.

use serde_json::Value;

use crate::time::short_in;

fn number(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64).filter(|n| n.is_finite())
}

fn text(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// `Math.round(n).toLocaleString("en-US")`.
pub fn credits(value: f64) -> String {
    let rounded = (value + 0.5).floor() as i64;
    let digits = rounded.unsigned_abs().to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    if rounded < 0 {
        out.insert(0, '-');
    }
    out
}

/// Left-aligned first column, right-aligned numbers, three spaces apart.
fn table(header: [&str; 3], rows: Vec<[String; 3]>) -> Vec<String> {
    let all: Vec<[String; 3]> = std::iter::once(header.map(str::to_string))
        .chain(rows)
        .collect();
    let widths: Vec<usize> = (0..3)
        .map(|i| all.iter().map(|r| r[i].chars().count()).max().unwrap_or(0))
        .collect();
    all.iter()
        .map(|row| {
            let cells: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(i, cell)| {
                    let pad = " ".repeat(widths[i] - cell.chars().count());
                    if i == 0 {
                        format!("{cell}{pad}")
                    } else {
                        format!("{pad}{cell}")
                    }
                })
                .collect();
            format!("  {}", cells.join("   "))
        })
        .collect()
}

/// `anysola usage` and the TUI's `/usage`: totals, then by day and by model.
pub fn usage_lines<Tz: chrono::TimeZone>(body: &Value, zone: &Tz) -> Vec<String>
where
    Tz::Offset: std::fmt::Display,
{
    let count = |row: &Value, key: &str| credits(number(row.get(key)).unwrap_or(0.0));
    let rows = |key: &str, name: &str| -> Vec<[String; 3]> {
        body.get(key)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        let label = item.get(name).and_then(Value::as_str).unwrap_or_default();
                        [
                            label.to_string(),
                            count(item, "requests"),
                            count(item, "credits"),
                        ]
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let totals = body.get("totals").cloned().unwrap_or(Value::Null);
    let mut lines = vec!["Totals".to_string()];
    lines.extend(table(
        ["", "requests", "credits"],
        vec![[
            "all".into(),
            count(&totals, "requests"),
            count(&totals, "credits"),
        ]],
    ));
    if let Some(rate) = totals.get("cacheHitRate").and_then(Value::as_f64) {
        lines.push(format!(
            "  Cache hit rate {}%",
            (rate * 100.0 + 0.5).floor()
        ));
    }
    lines.extend(["".into(), "By day".into()]);
    lines.extend(table(
        ["date", "requests", "credits"],
        rows("byDay", "date"),
    ));
    lines.extend(["".into(), "By model".into()]);
    lines.extend(table(
        ["model", "requests", "credits"],
        rows("byModel", "model"),
    ));
    if let Some(available) = body.get("creditsAvailable").and_then(Value::as_f64) {
        let renews = text(body.get("renewsAt"))
            .and_then(|at| short_in(at, zone))
            .map(|at| format!(" · renews {at}"))
            .unwrap_or_default();
        lines.extend([
            String::new(),
            format!("Balance {} credits{renews}", credits(available)),
        ]);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_numbers() {
        assert_eq!(credits(1234.0), "1,234");
        assert_eq!(credits(1234567.4), "1,234,567");
        assert_eq!(credits(999.5), "1,000");
        assert_eq!(credits(0.0), "0");
        assert_eq!(credits(-1500.0), "-1,500");
    }
}
