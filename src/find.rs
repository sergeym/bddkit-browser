//! Mink-style element lookup: what `I follow "Sign in"` means in XPath.
//! Every lookup is a list of strategies tried in order; the first non-empty
//! answer wins. No JavaScript — everything goes through WebDriver `find`.

use std::time::{Duration, Instant};

use crate::webdriver::{Element, Error, Session, Strategy};

/// An XPath string literal for `s`, whatever quotes it contains.
pub fn xpath_literal(s: &str) -> String {
    if !s.contains('\'') {
        format!("'{s}'")
    } else if !s.contains('"') {
        format!("\"{s}\"")
    } else {
        let parts: Vec<String> = s.split('\'').map(|p| format!("'{p}'")).collect();
        format!("concat({})", parts.join(", \"'\", "))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lookup {
    /// For messages: `link "Sign in"`, `element "#cart"`.
    pub what: String,
    pub strategies: Vec<(Strategy, String)>,
}

/// A `<selector>` argument: CSS by default, `xpath=…`, or `text=…` for an
/// element whose own text nodes contain the string (`body` would otherwise
/// match first).
pub fn selector(raw: &str) -> Lookup {
    let strategy = if let Some(xpath) = raw.strip_prefix("xpath=") {
        (Strategy::XPath, xpath.to_string())
    } else if let Some(text) = raw.strip_prefix("text=") {
        (
            Strategy::XPath,
            format!(
                "//*[text()[contains(normalize-space(.), {})]]",
                xpath_literal(text)
            ),
        )
    } else {
        (Strategy::Css, raw.to_string())
    };
    Lookup {
        what: format!("element {raw:?}"),
        strategies: vec![strategy],
    }
}

pub fn link(text: &str) -> Lookup {
    let x = xpath_literal(text);
    Lookup {
        what: format!("link {text:?}"),
        strategies: vec![
            (
                Strategy::XPath,
                format!("//a[normalize-space(.)={x} or @title={x} or @id={x} or .//img[@alt={x}]]"),
            ),
            (
                Strategy::XPath,
                format!("//a[contains(normalize-space(.), {x})]"),
            ),
            (Strategy::Css, text.to_string()),
        ],
    }
}

const BUTTON_INPUT: &str = "(@type='submit' or @type='button' or @type='reset' or @type='image')";

pub fn button(text: &str) -> Lookup {
    let x = xpath_literal(text);
    Lookup {
        what: format!("button {text:?}"),
        strategies: vec![
            (
                Strategy::XPath,
                format!(
                    "//button[normalize-space(.)={x} or @value={x} or @id={x} or @name={x} or @title={x}] \
                     | //input[{BUTTON_INPUT} and (@value={x} or @id={x} or @name={x} or @title={x})]"
                ),
            ),
            (
                Strategy::XPath,
                format!(
                    "//button[contains(normalize-space(.), {x})] | //input[{BUTTON_INPUT} and contains(@value, {x})]"
                ),
            ),
            (Strategy::Css, text.to_string()),
        ],
    }
}

/// `input` (not a button), `textarea` or `select`.
const FIELD: &str = "//*[(self::input and not(@type='submit' or @type='button' or @type='reset' or @type='image')) or self::textarea or self::select]";
const FIELD_INSIDE_LABEL: &str = ".//*[self::input or self::select or self::textarea]";

pub fn field(text: &str) -> Lookup {
    let x = xpath_literal(text);
    Lookup {
        what: format!("field {text:?}"),
        strategies: vec![
            (
                Strategy::XPath,
                format!("{FIELD}[@id={x} or @name={x} or @placeholder={x}]"),
            ),
            (
                Strategy::XPath,
                format!(
                    "{FIELD}[@id=//label[normalize-space(.)={x}]/@for] | //label[normalize-space(.)={x}]{}",
                    FIELD_INSIDE_LABEL.trim_start_matches('.')
                ),
            ),
            (
                Strategy::XPath,
                format!(
                    "{FIELD}[@id=//label[contains(normalize-space(.), {x})]/@for] | //label[contains(normalize-space(.), {x})]{}",
                    FIELD_INSIDE_LABEL.trim_start_matches('.')
                ),
            ),
            (Strategy::Css, text.to_string()),
        ],
    }
}

/// Relative to the found `select`.
pub fn option_xpath(text: &str) -> String {
    let x = xpath_literal(text);
    format!(".//option[normalize-space(.)={x} or @value={x}]")
}

/// One look: the first strategy with an answer.
pub fn find_once<'a>(session: &'a Session, lookup: &Lookup) -> Result<Option<Element<'a>>, Error> {
    for (strategy, value) in &lookup.strategies {
        if let Some(element) = session.find(*strategy, value)? {
            return Ok(Some(element));
        }
    }
    Ok(None)
}

/// An action's wait: poll every 100 ms up to `timeout`. This is the one
/// `sleep` in the plugin — clicking a button that is about to render is the
/// ordinary case, not the eventual one. Assertions never come here.
pub fn wait_for<'a>(
    session: &'a Session,
    lookup: &Lookup,
    timeout: Duration,
) -> Result<Option<Element<'a>>, Error> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(element) = find_once(session, lookup)? {
            return Ok(Some(element));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xpath_literals_survive_both_kinds_of_quote() {
        assert_eq!(xpath_literal("plain"), "'plain'");
        assert_eq!(xpath_literal("it's"), "\"it's\"");
        assert_eq!(
            xpath_literal(r#"say "it's""#),
            r#"concat('say "it', "'", 's"')"#
        );
    }

    #[test]
    fn a_selector_defaults_to_css_and_honours_the_prefixes() {
        assert_eq!(
            selector("#cart").strategies,
            vec![(Strategy::Css, "#cart".to_string())]
        );
        assert_eq!(
            selector("xpath=//a[1]").strategies,
            vec![(Strategy::XPath, "//a[1]".to_string())]
        );
        let by_text = selector("text=Pay now");
        assert_eq!(by_text.strategies[0].0, Strategy::XPath);
        assert!(
            by_text.strategies[0]
                .1
                .contains("text()[contains(normalize-space(.), 'Pay now')]")
        );
    }

    #[test]
    fn named_lookups_try_exact_then_partial_then_css() {
        let l = link("Sign in");
        assert_eq!(l.strategies.len(), 3);
        assert!(l.strategies[0].1.contains("normalize-space(.)='Sign in'"));
        assert!(
            l.strategies[1]
                .1
                .contains("contains(normalize-space(.), 'Sign in')")
        );
        assert_eq!(l.strategies[2], (Strategy::Css, "Sign in".to_string()));
        assert_eq!(l.what, "link \"Sign in\"");

        let b = button("Pay");
        assert!(
            b.strategies[0].1.contains("//button[") && b.strategies[0].1.contains("@type='submit'")
        );

        let f = field("E-mail");
        assert!(f.strategies[0].1.contains("@placeholder='E-mail'"));
        assert!(
            f.strategies[1]
                .1
                .contains("//label[normalize-space(.)='E-mail']/@for")
        );
        assert!(
            f.strategies[2]
                .1
                .contains("//label[contains(normalize-space(.), 'E-mail')]")
        );
        assert_eq!(
            f.strategies.last(),
            Some(&(Strategy::Css, "E-mail".to_string()))
        );

        assert_eq!(
            option_xpath("Latvia"),
            ".//option[normalize-space(.)='Latvia' or @value='Latvia']"
        );
    }
}
