use edit_time_check::{changed_ranges, decode_request, Operation};

const REQUEST: &str = r#"{"version":1,"request_id":"fixture-1","operation":"edit","path":"/tmp/source.rs","repository_root":null,"original_text":"x","proposed_text":"y","changed_ranges":[],"budget_ms":3000}"#;

#[test]
fn strict_protocol_rejects_untrusted_shapes() {
    assert!(decode_request(REQUEST.as_bytes()).is_ok());
    for text in [
        REQUEST.replace("\"version\":1", "\"version\":2"),
        REQUEST.replace("\"version\":1", "\"version\":1,\"version\":1"),
        REQUEST.replace("\"version\":1", "\"version\":1,\"command\":\"true\""),
        REQUEST.replace("fixture-1", ""),
        REQUEST.replace("/tmp/source.rs", "relative.rs"),
        REQUEST.replace("\"budget_ms\":3000", "\"budget_ms\":0"),
        REQUEST.replace("\"original_text\":\"x\"", "\"original_text\":null"),
        REQUEST.replace(
            "\"changed_ranges\":[]",
            "\"changed_ranges\":[{\"start_byte\":2,\"end_byte\":3}]",
        ),
        format!("{REQUEST}\n{REQUEST}"),
    ] {
        assert!(decode_request(text.as_bytes()).is_err(), "{text}");
    }
    assert!(decode_request(&[255]).is_err());
}

#[test]
fn actual_changes_ignore_claims_and_keep_deletions_and_unicode() {
    let ranges = changed_ranges(Some("a\nsecret\nz\n"), "b\nsecret\ny\n", Operation::Edit);
    assert!(ranges.iter().all(|r| r.end_byte <= 2 || r.start_byte >= 9));
    let deletion = changed_ranges(Some("a\nb\nc\n"), "a\nc\n", Operation::Edit);
    assert!(deletion
        .iter()
        .any(|r| r.start_byte == 2 && r.end_byte == 2));
    let unicode = changed_ranges(Some("α"), "β", Operation::Edit);
    assert_eq!((unicode[0].start_byte, unicode[0].end_byte), (0, 2));
    assert!(changed_ranges(Some("same"), "same", Operation::Edit).is_empty());
    let write = changed_ranges(Some("same"), "same", Operation::Write);
    assert_eq!((write[0].start_byte, write[0].end_byte), (0, 4));
}

#[test]
fn checker_category_is_not_a_configurable_rule_or_rule_violation() {
    use edit_time_check::{Decision, DiagnosticRule, Response, Rule};
    assert!(serde_json::from_str::<Rule>("\"checker\"").is_err());
    let mut response = Response::error(
        "fixture".into(),
        "/tmp/x.rs".into(),
        DiagnosticRule::CHECKER,
        "cannot read configuration".into(),
    );
    let encoded = serde_json::to_value(&response).unwrap();
    assert_eq!(encoded["version"], 1);
    assert_eq!(encoded["diagnostics"][0]["rule"], "checker");
    assert!(serde_json::from_value::<Response>(encoded)
        .unwrap()
        .is_valid());
    response.decision = Decision::Block;
    assert!(!response.is_valid());
    let response = Response::error(
        "fixture".into(),
        "/tmp/x.rs".into(),
        Rule::BooleanName,
        "rule deadline exceeded".into(),
    );
    assert_eq!(
        serde_json::to_value(response).unwrap()["diagnostics"][0]["rule"],
        "boolean-name"
    );
}

#[test]
fn line_refinement_keeps_disjoint_unicode_changes_and_deletions() {
    let old = "fn ready() -> bool { α == β }\nunchanged\nfn other() { γ(); }\n";
    let new = "fn is_ready() -> bool { α == δ }\nunchanged\nfn other() {  }\n";
    let ranges = changed_ranges(Some(old), new, Operation::Edit);
    assert!(ranges.iter().any(|r| r.start_byte == r.end_byte));
    for range in &ranges {
        assert!(new.is_char_boundary(range.start_byte));
        assert!(new.is_char_boundary(range.end_byte));
        assert!(!new[range.start_byte..range.end_byte].contains("unchanged"));
    }
    assert!(ranges
        .windows(2)
        .all(|pair| pair[0].end_byte <= pair[1].start_byte));
    assert!(ranges.len() >= 3);
}

#[test]
fn boolean_evidence_is_not_truthiness_or_an_annotation_exemption() {
    for (path, text, count) in [
        (
            "x.rs",
            "fn f(ready: bool) -> bool { let done = true; ready }",
            3,
        ),
        ("x.rs", "struct S { ready: bool }", 1),
        (
            "x.ts",
            "const ready = true; const value = input || fallback;",
            1,
        ),
        (
            "x.ts",
            "const ready = value === other; const isReady = false;",
            1,
        ),
        ("x.ts", "class X { @external ready: boolean = true; }", 1),
        ("x.ts", "const local = remote.ready;", 0),
        ("x.py", "ready = True\nvalue = input or fallback\n", 1),
        ("x.swift", "let ready: Bool = true", 0),
    ] {
        let findings = edit_time_check::boolean_findings(path, text).unwrap();
        assert_eq!(findings.len(), count, "{path}: {text}: {findings:?}");
    }
}
