use latex2mathml::{DisplayStyle, latex_to_mathml};
use scraper::{ElementRef, Html};

pub(super) fn html(source: &str, display: bool) -> String {
    let text = render(source)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    if display {
        format!("<div data-anotau-math=\"block\">{text}</div>")
    } else {
        format!("<span data-anotau-math=\"inline\">{text}</span>")
    }
}

pub(super) fn render(source: &str) -> String {
    // acota el trabajo del parser sin perder la fuente de una fórmula incompleta.
    let mut depth = 0usize;
    // latex2mathml 0.2.3 tiene una rama no implementada para \middle.
    if source.len() > 2048
        || source.matches(['\\', '^', '_']).count() > 128
        || source.contains(r"\middle")
    {
        return source.into();
    }
    for ch in source.chars() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth > 32 {
            return source.into();
        }
    }
    let Ok(mathml) = latex_to_mathml(source, DisplayStyle::Inline) else {
        return source.into();
    };
    if mathml.contains("[PARSE ERROR") {
        return source.into();
    }
    Html::parse_fragment(&mathml)
        .root_element()
        .child_elements()
        .map(expression)
        .collect()
}

fn expression(element: ElementRef<'_>) -> String {
    let children = element.child_elements().map(expression).collect::<Vec<_>>();
    let child = |index| children.get(index).map_or("", String::as_str);
    match element.value().name() {
        "msup" | "mover" => format!("{}{}", child(0), script(child(1), true)),
        "msub" | "munder" => format!("{}{}", child(0), script(child(1), false)),
        "msubsup" | "munderover" => format!(
            "{}{}{}",
            child(0),
            script(child(1), false),
            script(child(2), true)
        ),
        "mfrac" => format!("({})/({})", child(0), child(1)),
        "msqrt" => format!("√({})", children.concat()),
        "mroot" => format!("root({}, {})", child(1), child(0)),
        "mspace" => " ".into(),
        "mtable" => format!("[{}]", children.join("; ")),
        "mtr" => children.join(", "),
        _ if children.is_empty() => element.text().collect(),
        _ => children.concat(),
    }
}

fn script(text: &str, upper: bool) -> String {
    let (ascii, unicode) = if upper {
        ("0123456789+-=()in", "⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻⁼⁽⁾ⁱⁿ")
    } else {
        (
            "0123456789+-=()aehijklmnoprstuvx",
            "₀₁₂₃₄₅₆₇₈₉₊₋₌₍₎ₐₑₕᵢⱼₖₗₘₙₒₚᵣₛₜᵤᵥₓ",
        )
    };
    text.chars()
        .map(|ch| {
            ascii
                .chars()
                .zip(unicode.chars())
                .find_map(|(from, to)| (from == ch).then_some(to))
        })
        .collect::<Option<String>>()
        .unwrap_or_else(|| format!("{}({text})", if upper { '^' } else { '_' }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_expressions_use_unicode_and_invalid_input_stays_readable() {
        assert_eq!(render("a^2 + b^2 = c^2"), "a²+b²=c²");
        let integral = render(r"\int_0^1 x^2\,dx = \frac{1}{3}");
        assert!(
            integral.contains("∫₀¹") && integral.contains("x²") && integral.contains("(1)/(3)"),
            "{integral}"
        );
        assert_eq!(render(r"\sqrt{\alpha + 1}"), "√(α+1)");
        for input in [
            r"\unknown{a}",
            r"\frac{",
            r"\middle x",
            r"\middle{a}",
            r"\left( x \middle x \right)",
            &"{".repeat(33),
        ] {
            assert_eq!(render(input), input);
        }
    }
}
