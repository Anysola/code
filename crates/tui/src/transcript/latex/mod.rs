//! ```` ```latex ````/```` ```math ```` blocks as 2-D Unicode math, a port of 0.5.1's
//! `@opencode/latex` in the mode the transcript uses it (strict, display, compact scripts):
//! anything it cannot parse is an error and the block stays source.

mod layout;
mod parser;
mod symbols;

const MAX_SOURCE: usize = 100_000;

/// The formula's rows, or why it cannot be shown.
pub fn render(source: &str) -> Result<Vec<String>, String> {
    render_with(source, true, true)
}

fn render_with(source: &str, display: bool, compact: bool) -> Result<Vec<String>, String> {
    if source.len() > MAX_SOURCE {
        return Err("source too long".into());
    }
    let source = source.trim();
    if source.is_empty() {
        return Err("empty".into());
    }
    let node = parser::Parser::new(source).parse()?;
    Ok(layout::layout(&node, layout::Context::new(display, compact)).rows())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(source: &str) -> String {
        render(source).unwrap().join("\n")
    }

    /// `render.test.ts`.
    #[test]
    fn renders_like_opencode_latex() {
        assert_eq!(text(r"\frac{x+1}{y-1}"), " x + 1\n───────\n y - 1");
        for (source, expected) in [
            (r"E = mc^2", "E = mc²"),
            (r"a_n", "aₙ"),
            (r"x_i^2", "x²ᵢ"),
            (r"x^{}", "x"),
            (r"x_{}", "x"),
            (r"x^{}_{}", "x"),
            (r"x^m_1", " m\nx\n 1"),
            (r"x^2_q", " 2\nx\n q"),
            (r"x^{\frac{1}{2}}_1", "  1\n ───\n  2\nx\n 1"),
        ] {
            assert_eq!(text(source), expected, "{source}");
        }
        assert_eq!(
            render_with(r"x_i^2", true, false).unwrap().join("\n"),
            " 2\nx\n i"
        );
        assert_eq!(
            render_with(r"\sum_1^n", false, true).unwrap().join("\n"),
            "∑ⁿ₁"
        );
        assert_eq!(
            render_with(r"\sum_1^n", true, false).unwrap().join("\n"),
            "n\n∑\n1"
        );
        assert_eq!(text(r"P = \binom{n}{k}"), "    ⎛ n ⎞\nP = ⎜   ⎟\n    ⎝ k ⎠");
        assert_eq!(text(r"\sqrt{x^2+y^2}"), " ╭───────\n╰╯x² + y²");
        assert_eq!(
            text(r"\begin{pmatrix}a & b \\ c & d\end{pmatrix}"),
            "⎛a b⎞\n⎜   ⎟\n⎝c d⎠"
        );
        assert_eq!(text(r"\sum_{i=1}^{n} i^2"), "  n\n  ∑   i²\ni = 1");
        assert_eq!(
            text(r"\mathbb{R} \to \mathcal{C} \times \mathfrak{g}"),
            "ℝ → 𝒞 × 𝔤"
        );
        let nested = text(r"\frac{1}{1+\frac{1}{x}}");
        assert_eq!(nested.lines().count(), 5);
        assert!(nested.matches('─').count() >= 10);
        let textbook = text(r"\left[\frac{-b \pm \sqrt{b^2-4ac}}{2a}\right]");
        for needle in ["±", "╰╯", "─", "⎡", "⎦"] {
            assert!(textbook.contains(needle), "{needle}\n{textbook}");
        }
        assert_eq!(text(r"\not\rightarrow"), "→\u{338}");
        assert_eq!(text(r"x\in[0,1]"), "x ∈ [0,1]");
        assert_eq!(text(r"[-1,1]"), "[-1,1]");
    }

    #[test]
    fn geometry_and_baseline() {
        let node = parser::Parser::new(r"\frac{1}{2}").parse().unwrap();
        let block = layout::layout(&node, layout::Context::new(true, true));
        assert_eq!((block.width, block.height, block.baseline), (3, 3, 1));
    }

    /// `layout.test.ts`.
    #[test]
    fn structured_layout() {
        for source in [
            r"\sqrt{x}",
            r"\begin{pmatrix}a&b\\c&d\end{pmatrix}",
            r"\underbrace{abcd}",
            r"\overbrace{abcd}",
            r"\sum",
        ] {
            for scripts in ["^{}", "_{}", "^{}_{}"] {
                assert_eq!(
                    text(&format!("{source}{scripts}")),
                    text(source),
                    "{source}{scripts}"
                );
            }
        }
        assert_eq!(text(r"\overbrace{abcd}^{n}"), " n\n╭┴─╮\nabcd");
        assert_eq!(text(r"\underbrace{abcd}_{n}"), "abcd\n╰┬─╯\n n");
        assert_eq!(
            text(r"\begin{pmatrix}a&b\\c&d\end{pmatrix}^2"),
            "     2\n⎛a b⎞\n⎜   ⎟\n⎝c d⎠"
        );
        assert_eq!(
            text(r"\begin{cases}x & x>0\\x^2+1 & x\le0\end{cases}"),
            "⎧x       x > 0\n⎨\n⎩x² + 1  x ≤ 0"
        );
        assert_eq!(
            text(r"\begin{array}{l|r}a & wide\\long & b\end{array}"),
            "a    │ wide\n     │\nlong │    b"
        );
        assert_eq!(
            text(r"\begin{array}{|l||r|}a & b\\long & c\end{array}"),
            "│ a    ││ b │\n│      ││   │\n│ long ││ c │"
        );
        assert_eq!(text(r"\cfrac[l]{1}{12345}"), " 1\n───────\n 12345");
        assert_eq!(text(r"\cfrac[r]{1}{12345}"), "     1\n───────\n 12345");
        assert_eq!(
            text(r"\overbrace{a + b + c}^{n}"),
            "    n\n╭───┴───╮\na + b + c"
        );
    }

    /// `root.test.ts`.
    #[test]
    fn roots() {
        assert_eq!(
            text(r"\sqrt{\sqrt{\sqrt{x}}}"),
            " ╭─────\n │ ╭───\n │ │ ╭─\n╰╯╰╯╰╯x"
        );
        assert_eq!(text(r"\sqrt{\frac{a}{b}}"), " ╭───\n │ a\n │───\n╰╯ b");
        assert_eq!(text(r"\sqrt[3]{x}"), "3╭─\n╰╯x");
        assert_eq!(text(r"\sqrt[\frac{1}{2}]{x}"), " 1\n───\n 2 ╭─\n  ╰╯x");
        assert_eq!(text(r"\sqrt[\sqrt{n}]{x}"), " ╭─\n╰╯n╭─\n  ╰╯x");
    }

    /// `parser-render.test.ts`.
    #[test]
    fn parser_regressions() {
        for (source, expected) in [
            (r"\|v\|", "║v║"),
            (r"\left\|v\right\|", "║v║"),
            (r"\left|v\right|", "│v│"),
            (r"\left(A\rightarrow B\right)", "(A → B)"),
            (r"\left\lbrace x\right\rbrace", "{x}"),
            (r"\operatorname{arg\,max} x", "arg max x"),
            (r"\textrm{if }x", "if x"),
            (r"\displaylines{x=1\\y=2}", "x = 1\n\ny = 2"),
        ] {
            assert_eq!(text(source), expected, "{source}");
        }
        assert_eq!(
            text(r"\begin{aligned}&=x\\&=y\end{aligned}"),
            text(r"\begin{aligned}{}&=x\\{}&=y\end{aligned}")
        );
    }

    #[test]
    fn unsupported_input_falls_back() {
        assert!(render(r"\frac{a}{").is_err());
        assert!(render(r"\unknowncommand x").is_err());
        assert!(render(r"\begin{tikzpicture}\end{tikzpicture}").is_err());
    }
}
