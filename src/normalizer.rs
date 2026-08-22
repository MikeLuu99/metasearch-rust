use url::Url;

// Query parameters added by ad/analytics platforms that don't affect page content.
// Two URLs differing only by these params are the same page for deduplication purposes.
const TRACKING_PARAMS: &[&str] = &[
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "fbclid",
    "gclid",
    "msclkid",
    "yclid",
    "ref",
    "source",
];

// Index filenames that are semantically equivalent to the directory path.
// /page/index.html and /page/ resolve to the same content on virtually all servers.
const INDEX_FILES: &[&str] = &["index.html", "index.htm", "index.php"];

/// Normalize a URL to a canonical string used as the deduplication key.
///
/// Two raw URLs referring to the same page should produce the same key.
/// Returns None if the URL cannot be parsed — callers should skip those results.
pub fn normalize(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw).ok()?;

    url.set_fragment(None);

    let clean: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| !TRACKING_PARAMS.contains(&k.as_ref()))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    if clean.is_empty() {
        url.set_query(None);
    } else {
        let mut sorted = clean;
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        url.query_pairs_mut().clear().extend_pairs(sorted);
    }

    let path = url.path().to_string();
    let path = strip_locale_prefix(&path);
    let path = strip_index_file(path);

    // Strip trailing slash from non-root paths (/page/ → /page, / stays /)
    let path = if path.len() > 1 && path.ends_with('/') {
        path.trim_end_matches('/').to_string()
    } else {
        path.to_string()
    };

    url.set_path(&path);

    // The url crate already lowercases scheme and host at parse time.
    // Lowercasing the full string here would merge case-sensitive paths
    // (e.g. /RustBook vs /rustbook) and query values into one key.
    Some(url.to_string())
}

/// Strip a leading locale segment from a URL path.
///
/// Matches 2-letter language codes (e.g. `/en/`) and language-region codes
/// (e.g. `/en-US/`, `/en_US/`) only when followed by another slash, so that
/// short but legitimate path segments like `/go` are left untouched.
fn strip_locale_prefix(path: &str) -> &str {
    let rest = match path.strip_prefix('/') {
        Some(r) => r,
        None => return path,
    };

    // Require a trailing slash after the segment — /en/docs not /en (bare segment)
    let (segment, remainder) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => return path,
    };

    if is_locale_segment(segment) {
        remainder
    } else {
        path
    }
}

/// Returns true if `s` is a known locale code used as a path prefix.
/// Matches 2-letter ISO 639-1 language codes (e.g. `en`, `fr`) and language-region
/// codes (e.g. `en-US`, `en_US`, `zh-CN`, `pt-BR`) whose language part is a known
/// code. Anchoring on the ISO list keeps legitimate 2-letter path segments like
/// `/go` or `/us` from being mistaken for locales.
fn is_locale_segment(s: &str) -> bool {
    let b = s.as_bytes();
    match b.len() {
        2 => is_iso_639_1(s),
        5 => {
            b[0].is_ascii_alphabetic()
                && b[1].is_ascii_alphabetic()
                && (b[2] == b'-' || b[2] == b'_')
                && b[3].is_ascii_alphabetic()
                && b[4].is_ascii_alphabetic()
                && is_iso_639_1(&s[..2])
        }
        _ => false,
    }
}

/// True if `s` is a 2-letter ISO 639-1 language code.
fn is_iso_639_1(s: &str) -> bool {
    ISO_639_1.binary_search(&s).is_ok()
}

// All ISO 639-1 language codes, sorted for binary_search.
const ISO_639_1: &[&str] = &[
    "aa", "ab", "ae", "af", "ak", "am", "an", "ar", "as", "av", "ay", "az", "ba", "be", "bg", "bh",
    "bi", "bm", "bn", "bo", "br", "bs", "ca", "ce", "ch", "co", "cr", "cs", "cu", "cv", "cy", "da",
    "de", "dv", "dz", "ee", "el", "en", "eo", "es", "et", "eu", "fa", "ff", "fi", "fj", "fo", "fr",
    "fy", "ga", "gd", "gl", "gn", "gu", "gv", "ha", "he", "hi", "ho", "hr", "ht", "hu", "hy", "hz",
    "ia", "id", "ie", "ig", "ii", "ik", "io", "is", "it", "iu", "ja", "jv", "ka", "kg", "ki", "kj",
    "kk", "kl", "km", "kn", "ko", "kr", "ks", "ku", "kv", "kw", "ky", "la", "lb", "lg", "li", "ln",
    "lo", "lt", "lu", "lv", "mg", "mh", "mi", "mk", "ml", "mn", "mr", "ms", "mt", "my", "na", "nb",
    "nd", "ne", "ng", "nl", "nn", "no", "nr", "nv", "ny", "oc", "oj", "om", "or", "os", "pa", "pi",
    "pl", "ps", "pt", "qu", "rm", "rn", "ro", "ru", "rw", "sa", "sc", "sd", "se", "sg", "si", "sk",
    "sl", "sm", "sn", "so", "sq", "sr", "ss", "st", "su", "sv", "sw", "ta", "te", "tg", "th", "ti",
    "tk", "tl", "tn", "to", "tr", "ts", "tt", "tw", "ty", "ug", "uk", "ur", "uz", "ve", "vi", "vo",
    "wa", "wo", "xh", "yi", "yo", "za", "zh", "zu",
];

/// Strip index filenames so /page/index.html and /page/ produce the same path.
/// The suffix must be preceded by `/` so pages like /docs/reindex.html are
/// left untouched instead of collapsing to /docs/re.
fn strip_index_file(path: &str) -> &str {
    for index in INDEX_FILES {
        if let Some(dir) = path.strip_suffix(index)
            && dir.ends_with('/')
        {
            return dir;
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_removes_tracking_params() {
        let n = normalize("https://example.com/page?utm_source=google&q=rust").unwrap();
        assert!(!n.contains("utm_source"));
        assert!(n.contains("q=rust"));
    }

    #[test]
    fn test_removes_fragment() {
        let a = normalize("https://example.com/page#section").unwrap();
        let b = normalize("https://example.com/page").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_removes_trailing_slash() {
        let a = normalize("https://example.com/page/").unwrap();
        let b = normalize("https://example.com/page").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_root_slash_preserved() {
        let n = normalize("https://example.com/").unwrap();
        assert!(n.ends_with('/') || n == "https://example.com");
    }

    #[test]
    fn test_sorts_query_params() {
        let a = normalize("https://example.com/?z=1&a=2").unwrap();
        let b = normalize("https://example.com/?a=2&z=1").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_lowercases_scheme_and_host() {
        let a = normalize("HTTPS://Example.COM/page").unwrap();
        let b = normalize("https://example.com/page").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_preserves_path_case() {
        let a = normalize("https://example.com/RustBook").unwrap();
        let b = normalize("https://example.com/rustbook").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn test_preserves_query_value_case() {
        let a = normalize("https://example.com/?q=RustBook").unwrap();
        let b = normalize("https://example.com/?q=rustbook").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn test_returns_none_for_invalid_url() {
        assert!(normalize("not a url").is_none());
    }

    #[test]
    fn test_strips_locale_language_only() {
        let a = normalize("https://example.com/en/docs").unwrap();
        let b = normalize("https://example.com/docs").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_strips_locale_language_region_hyphen() {
        let a = normalize("https://rust-lang.org/en-US/").unwrap();
        let b = normalize("https://rust-lang.org/").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_strips_locale_language_region_underscore() {
        let a = normalize("https://example.com/en_US/page").unwrap();
        let b = normalize("https://example.com/page").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_does_not_strip_bare_short_segment() {
        // /go with no trailing slash — strip_locale_prefix requires a following /
        // so this is left untouched even though "go" is 2 letters
        let n = normalize("https://example.com/go").unwrap();
        assert!(n.contains("/go"));
    }

    #[test]
    fn test_does_not_strip_non_locale_segment() {
        // "go" is not an ISO 639-1 code — a real path segment must survive
        let n = normalize("https://github.com/go/website").unwrap();
        assert!(n.contains("/go/"));
    }

    #[test]
    fn test_encodes_special_chars_in_query_values() {
        // A value containing & or = must not merge into separate query params
        let a = normalize("https://example.com/?a=1%26b%3D2").unwrap();
        let b = normalize("https://example.com/?a=1&b=2").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn test_strips_index_html() {
        let a = normalize("https://example.com/page/index.html").unwrap();
        let b = normalize("https://example.com/page").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_strips_index_htm() {
        let a = normalize("https://example.com/page/index.htm").unwrap();
        let b = normalize("https://example.com/page").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_strips_index_php() {
        let a = normalize("https://example.com/page/index.php").unwrap();
        let b = normalize("https://example.com/page").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_combined_locale_and_index() {
        let a = normalize("https://example.com/en-US/page/index.html").unwrap();
        let b = normalize("https://example.com/page").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_does_not_strip_embedded_index_filename() {
        // "reindex.html" merely ends with "index.html" — the suffix must be
        // preceded by '/' or the page would collapse to /docs/re
        let a = normalize("https://example.com/docs/reindex.html").unwrap();
        let b = normalize("https://example.com/docs/re").unwrap();
        assert_ne!(a, b);
        assert!(a.contains("/docs/reindex.html"));
    }
}
