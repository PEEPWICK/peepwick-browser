//! The interface's translations, taken from `src/shared/i18n.ts` at build time, with the same
//! rules as its `translate`: "one|other" plurals chosen by `n` and `{name}` variables.

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::settings::{self, Lang};

type Dictionary = HashMap<String, String>;

static DICTIONARIES: LazyLock<HashMap<String, Dictionary>> = LazyLock::new(|| {
    serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/i18n.json")))
        .expect("translations are valid JSON")
});

/// A value for a `{name}` placeholder; numbers are written the way the language groups digits.
pub enum Var<'a> {
    Text(&'a str),
    Number(i64),
}

fn code(lang: Lang) -> &'static str {
    match lang {
        Lang::Tr => "tr",
        Lang::En => "en",
    }
}

pub fn format_number(lang: Lang, value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let separator = if lang == Lang::Tr { '.' } else { ',' };
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(separator);
        }
        out.push(c);
    }
    if value < 0 { format!("-{out}") } else { out }
}

pub fn translate(lang: Lang, key: &str, vars: &[(&str, Var)]) -> String {
    let lookup = |l: &str| DICTIONARIES.get(l).and_then(|d| d.get(key)).cloned();
    let mut text = lookup(code(lang))
        .or_else(|| lookup("tr"))
        .unwrap_or_else(|| key.to_string());
    if let Some((one, other)) = text.split_once('|') {
        let n_is_one = vars
            .iter()
            .any(|(name, value)| *name == "n" && matches!(value, Var::Number(1)));
        text = if n_is_one {
            one.to_string()
        } else {
            other.to_string()
        };
    }
    for (name, value) in vars {
        let value = match value {
            Var::Text(text) => text.to_string(),
            Var::Number(n) => format_number(lang, *n),
        };
        text = text.replace(&format!("{{{name}}}"), &value);
    }
    text
}

/// Lower case for matching, with the Turkish dotted and dotless i.
pub fn fold(text: &str, lang: Lang) -> String {
    match lang {
        Lang::Tr => text
            .chars()
            .map(|c| match c {
                'I' => 'ı',
                'İ' => 'i',
                c => c,
            })
            .collect::<String>()
            .to_lowercase(),
        Lang::En => text.to_lowercase(),
    }
}

/// In the language of the interface.
pub fn t(key: &str) -> String {
    translate(settings::get().language, key, &[])
}

pub fn tv(key: &str, vars: &[(&str, Var)]) -> String {
    translate(settings::get().language, key, vars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_like_the_ui() {
        assert_eq!(translate(Lang::Tr, "menu.newTab", &[]), "Yeni sekme");
        assert_eq!(
            translate(Lang::En, "task.more", &[("n", Var::Number(1))]),
            "+1 tab"
        );
        assert_eq!(
            translate(Lang::En, "task.more", &[("n", Var::Number(3))]),
            "+3 tabs"
        );
        assert_eq!(translate(Lang::Tr, "no.such.key", &[]), "no.such.key");
        assert_eq!(format_number(Lang::Tr, 1234567), "1.234.567");
        assert_eq!(format_number(Lang::En, -1234), "-1,234");
        assert_eq!(fold("IŞIK İzmir", Lang::Tr), "ışık izmir");
        assert_eq!(fold("Issue", Lang::En), "issue");
    }
}
