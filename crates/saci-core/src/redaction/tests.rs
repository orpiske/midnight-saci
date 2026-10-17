use super::*;
use crate::evidence::{HttpEvidence, HttpResponseEvidence, ShellEvidence, Termination};

fn redactor() -> Redactor {
    let mut redactor = Redactor::new();
    assert!(redactor.add("TOKEN", "s3cr3t-token"));
    assert!(redactor.add("TOKEN_PREFIX", "s3cr3t"));
    assert!(
        !redactor.add("PIN", "123"),
        "short values are not registered"
    );
    redactor
}

#[test]
fn secrets_are_replaced_by_named_markers() {
    let redactor = redactor();
    assert_eq!(
        redactor.redact("use s3cr3t-token and s3cr3t, not 123"),
        "use [REDACTED:TOKEN] and [REDACTED:TOKEN_PREFIX], not 123"
    );
    assert!(matches!(redactor.redact("nothing here"), Cow::Borrowed(_)));
    assert!(!format!("{redactor:?}").contains("s3cr3t"));
}

#[test]
fn json_strings_and_keys_are_redacted() {
    let mut value = serde_json::json!({"s3cr3t-token": ["a s3cr3t-token", 1, {"x": "s3cr3t"}]});
    redactor().redact_json(&mut value);
    assert_eq!(
        value,
        serde_json::json!({"[REDACTED:TOKEN]": ["a [REDACTED:TOKEN]", 1, {"x": "[REDACTED:TOKEN_PREFIX]"}]})
    );
}

#[test]
fn credential_headers_are_masked_whatever_their_value() {
    let mut headers = BTreeMap::from([
        (
            "Authorization".to_owned(),
            "Bearer unknown-value".to_owned(),
        ),
        ("Set-Cookie".to_owned(), "session=abc".to_owned()),
        ("X-Echo".to_owned(), "s3cr3t-token".to_owned()),
        ("Content-Type".to_owned(), "text/plain".to_owned()),
    ]);
    redactor().redact_headers(&mut headers);
    assert_eq!(headers["Authorization"], REDACTED);
    assert_eq!(headers["Set-Cookie"], REDACTED);
    assert_eq!(headers["X-Echo"], "[REDACTED:TOKEN]");
    assert_eq!(headers["Content-Type"], "text/plain");
}

#[test]
fn shell_evidence_is_redacted() {
    let mut shell = EvidenceRecord::Shell(ShellEvidence {
        command: "curl -H 'X: s3cr3t-token'".into(),
        stdout: CapturedText::complete("s3cr3t-token"),
        stderr: CapturedText::complete("ok"),
        exit_code: Some(0),
        termination: Termination::Exited,
        restarted: false,
        session_lost: false,
    });
    redactor().redact_evidence(&mut shell);
    let text = serde_json::to_string(&shell).unwrap();
    assert!(!text.contains("s3cr3t"), "{text}");
}

#[test]
fn http_evidence_is_redacted() {
    let mut http = EvidenceRecord::Http(HttpEvidence {
        method: "GET".into(),
        url: "https://h/?key=s3cr3t-token".into(),
        request_headers: BTreeMap::from([("x-api-key".into(), "zzzz".into())]),
        request_body: Some(CapturedText::complete("s3cr3t")),
        response: Some(HttpResponseEvidence {
            status: 200,
            headers: BTreeMap::from([("x-echo".into(), "s3cr3t".into())]),
            body: CapturedText::complete("s3cr3t-token"),
            body_error: Some("reset after s3cr3t".into()),
        }),
        transport_error: Some("failed with s3cr3t".into()),
    });
    redactor().redact_evidence(&mut http);
    let text = serde_json::to_string(&http).unwrap();
    assert!(!text.contains("s3cr3t") && !text.contains("zzzz"), "{text}");
}

#[test]
fn capture_bounds_output_and_marks_truncation() {
    let redactor = Redactor::new();
    let mut capture = OutputCapture::new(10, &redactor);
    capture.push(b"0123456789");
    capture.push(b"abcdef");
    let captured = capture.finish(&redactor);
    assert_eq!(captured.text, "0123456789");
    assert!(captured.truncated);
    assert_eq!(captured.total_bytes, 16);
    let small = capture_text("short", 10, &redactor);
    assert!(!small.truncated);
    assert_eq!(small.text, "short");
}

#[test]
fn secrets_straddling_the_limit_are_still_redacted() {
    let captured = capture_text("0123456s3cr3t-token tail", 10, &redactor());
    assert!(captured.truncated);
    assert!(!captured.text.contains("s3c"), "{}", captured.text);
    assert!(captured.text.starts_with("0123456"));
}

#[test]
fn truncation_respects_utf8_and_replaces_invalid_bytes() {
    let redactor = Redactor::new();
    let captured = capture_text("ééééé", 3, &redactor);
    assert_eq!(captured.text, "é");
    assert!(captured.truncated);
    let mut capture = OutputCapture::new(10, &redactor);
    capture.push(&[0x66, 0xff, 0x6f]);
    assert_eq!(capture.finish(&redactor).text, "f\u{fffd}o");
}

#[test]
fn rendered_text_states_truncation() {
    let captured = capture_text("0123456789abc", 4, &Redactor::new());
    assert_eq!(
        captured.rendered(),
        "0123\n[output truncated: showing 4 of 13 bytes]"
    );
    assert_eq!(CapturedText::complete("x").rendered(), "x");
}

#[test]
fn every_free_text_event_field_is_redacted() {
    use crate::event::EventKind;
    use crate::run::{ErrorKind, ExecutionError};
    use crate::status::Outcome;
    let redactor = redactor();
    let mut kinds = vec![
        EventKind::AttemptFinished {
            outcome: Outcome::Error,
            assessment: None,
            error: Some(ExecutionError::new(ErrorKind::Provider, "bad s3cr3t")),
        },
        EventKind::UnitSkipped {
            reason: "s3cr3t".into(),
        },
        EventKind::RunFinished {
            outcome: Outcome::Error,
            reason: "s3cr3t".into(),
        },
    ];
    for kind in &mut kinds {
        redactor.redact_event(kind);
    }
    let text = serde_json::to_string(&kinds).unwrap();
    assert!(!text.contains("s3cr3t"), "{text}");
}
