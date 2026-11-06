//! Masking of credentials that are not configured secrets.
//!
//! Fixed patterns with a low false positive rate find well-known credential
//! formats in any text. Each match becomes a marker that names its kind, so
//! the evidence shows that a credential was there without its value.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;

/// Prefix of the marker that replaces a credential.
pub const MARKER_PREFIX: &str = "[REDACTED:credential:";

/// Bytes that the longest credential pattern usually needs past a cut, so
/// [`OutputCapture`](super::OutputCapture) still finds a credential at its
/// limit.
pub const MARGIN: usize = 512;

/// `(kind, pattern)`. A pattern with a `keep` group masks only the text
/// outside the `keep` groups.
const PATTERNS: [(&str, &str); 8] = [
    (
        "private-key",
        r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|\z)",
    ),
    ("aws-access-key-id", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
    (
        "github-token",
        r"\b(?:gh[pousr]_[A-Za-z0-9]{36,255}|github_pat_[A-Za-z0-9_]{22,255})\b",
    ),
    ("gitlab-token", r"\bglpat-[A-Za-z0-9_-]{20,}"),
    ("slack-token", r"\bxox[abposr]-[A-Za-z0-9-]{10,}"),
    (
        "jwt",
        r"\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
    ),
    ("google-api-key", r"\bAIza[0-9A-Za-z_-]{35}"),
    (
        "url-password",
        r"(?P<keep>\b[A-Za-z][A-Za-z0-9+.-]*://[^/\s:@]+:)[^/\s@]+(?P<keep_end>@)",
    ),
];

/// The compiled patterns. Each pattern is a constant that a test compiles,
/// so none is dropped in practice.
static COMPILED: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    PATTERNS
        .iter()
        .filter_map(|(kind, pattern)| Regex::new(pattern).ok().map(|regex| (*kind, regex)))
        .collect()
});

/// Replaces each credential in `text` with a marker that names its kind.
#[must_use]
pub fn mask(text: &str) -> Cow<'_, str> {
    let mut result = Cow::Borrowed(text);
    for (kind, regex) in COMPILED.iter() {
        if !regex.is_match(&result) {
            continue;
        }
        let masked = regex.replace_all(&result, |captures: &regex::Captures<'_>| {
            let keep = captures.name("keep").map_or("", |m| m.as_str());
            let keep_end = captures.name("keep_end").map_or("", |m| m.as_str());
            format!("{keep}{MARKER_PREFIX}{kind}]{keep_end}")
        });
        result = Cow::Owned(masked.into_owned());
    }
    result
}

/// The credential kinds whose markers occur in `text`.
#[must_use]
pub fn kinds(text: &str) -> BTreeSet<String> {
    text.match_indices(MARKER_PREFIX)
        .filter_map(|(start, _)| {
            let rest = text.get(start + MARKER_PREFIX.len()..)?;
            let kind = &rest[..rest.find(']')?];
            PATTERNS
                .iter()
                .any(|(known, _)| *known == kind)
                .then(|| kind.to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles() {
        assert_eq!(COMPILED.len(), PATTERNS.len());
    }

    /// `(text, masked)` for each kind.
    const MASKED: [(&str, &str); 10] = [
        (
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEow\n-----END RSA PRIVATE KEY-----\nnext", // notsecret
            "[REDACTED:credential:private-key]\nnext",
        ),
        (
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNz (cut)", // notsecret
            "[REDACTED:credential:private-key]",
        ),
        (
            "key=AKIAIOSFODNN7EXAMPLE end", // notsecret
            "key=[REDACTED:credential:aws-access-key-id] end",
        ),
        (
            "token ghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789 ok", // notsecret
            "token [REDACTED:credential:github-token] ok",
        ),
        (
            "github_pat_11ABCDEFG0123456789_abcdefghijklmnop", // notsecret
            "[REDACTED:credential:github-token]",
        ),
        (
            "PRIVATE-TOKEN: glpat-abcdefghij0123456789", // notsecret
            "PRIVATE-TOKEN: [REDACTED:credential:gitlab-token]",
        ),
        (
            "xoxb-1234567890-abcdefghij", // notsecret
            "[REDACTED:credential:slack-token]",
        ),
        (
            r#"{"access_token":"eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ0ZXN0In0.c2lnbmF0dXJlLXZhbHVl"}"#, // notsecret
            r#"{"access_token":"[REDACTED:credential:jwt]"}"#,
        ),
        (
            "AIzaSyA-1234567890abcdefghijklmnopqrstu", // notsecret
            "[REDACTED:credential:google-api-key]",
        ),
        (
            "postgres://admin:hunter22@db:5432/app", // notsecret
            "postgres://admin:[REDACTED:credential:url-password]@db:5432/app",
        ),
    ];

    #[test]
    fn each_kind_is_masked() {
        for (text, expected) in MASKED {
            assert_eq!(mask(text), expected, "{text}");
        }
    }

    #[test]
    fn near_misses_are_kept() {
        let texts = [
            "-----BEGIN PUBLIC KEY-----\nMIIBIj\n-----END PUBLIC KEY-----",
            "-----BEGIN CERTIFICATE-----",
            "AKIA1234",                   // notsecret
            "XAKIAIOSFODNN7EXAMPLE",      // notsecret
            "ghp_short",                  // notsecret
            "glpat-short",                // notsecret
            "eyJhbGciOiJSUzI1NiJ9 alone", // notsecret
            "sha256:3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a",
            "quay.io/wanaku/router@sha256:0123456789abcdef",
            "https://example.com:8443/path",
            "ssh://git@github.com/orpiske/midnight-saci.git",
            "git@github.com:orpiske/midnight-saci.git",
            "user:pass but no scheme@host",
        ];
        for text in texts {
            assert!(matches!(mask(text), Cow::Borrowed(_)), "{text}");
        }
    }

    #[test]
    fn kinds_lists_known_markers_only() {
        let text = mask("AKIAIOSFODNN7EXAMPLE and xoxp-1234567890-abc and AKIAIOSFODNN7EXAMPLF"); // notsecret
        assert_eq!(
            kinds(&text).into_iter().collect::<Vec<_>>(),
            ["aws-access-key-id", "slack-token"]
        );
        assert!(kinds("[REDACTED:credential:made-up] [REDACTED:credential:").is_empty());
    }
}
