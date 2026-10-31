//! Search engines, as in `src/shared/search.ts`.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use crate::settings::SearchEngine;

/// What encodeURIComponent leaves alone.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

pub fn encode_component(text: &str) -> String {
    utf8_percent_encode(text, COMPONENT).to_string()
}

fn template(engine: SearchEngine) -> &'static str {
    match engine {
        SearchEngine::Duckduckgo => "https://duckduckgo.com/?q={q}",
        SearchEngine::Brave => "https://search.brave.com/search?q={q}",
        SearchEngine::Startpage => "https://www.startpage.com/do/search?q={q}",
        SearchEngine::Google => "https://www.google.com/search?q={q}",
        SearchEngine::Ecosia => "https://www.ecosia.org/search?q={q}",
        SearchEngine::Bing => "https://www.bing.com/search?q={q}",
    }
}

pub fn search_url(engine: SearchEngine, query: &str) -> String {
    template(engine).replace("{q}", &encode_component(query))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_encode_uri_component() {
        assert_eq!(
            encode_component("çay & simit (2)!"),
            "%C3%A7ay%20%26%20simit%20(2)!"
        );
        assert_eq!(
            search_url(SearchEngine::Duckduckgo, "a b"),
            "https://duckduckgo.com/?q=a%20b"
        );
    }
}
