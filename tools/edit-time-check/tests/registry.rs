use edit_time_check::{decode_request, evaluate, Decision, Options, Rule, Stage};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    options: Options,
}
impl Fixture {
    fn new(rules: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "edit-time-registry-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let root = root.canonicalize().unwrap();
        std::fs::write(root.join("config"), format!("version=1\nrules=[{rules}]\n")).unwrap();
        std::fs::write(root.join("comments"), "Only approved comment shapes.").unwrap();
        std::fs::write(root.join("code"), "Booleans use is prefix.").unwrap();
        let mut options = Options::for_paths(
            root.join("config"),
            root.join("comments"),
            root.join("code"),
        );
        options.judgment_configuration = "fixture-v1".into();
        Self { root, options }
    }
    fn run(&self, old: &str, new: &str, operation: &str) -> edit_time_check::Response {
        self.run_path("x.rs", old, new, operation)
    }
    fn run_path(
        &self,
        path: &str,
        old: &str,
        new: &str,
        operation: &str,
    ) -> edit_time_check::Response {
        let input = serde_json::json!({"version":1,"request_id":"fixture","operation":operation,"path":self.root.join(path),"repository_root":self.root,"original_text":old,"proposed_text":new,"changed_ranges":[],"budget_ms":20000});
        let request = decode_request(&serde_json::to_vec(&input).unwrap()).unwrap();
        let response = evaluate(&request, &self.options, &Stage::new());
        assert!(response.is_valid(), "{response:?}");
        response
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn boolean_changes_select_name_and_evidence_not_body() {
    let fixture = Fixture::new("'boolean-name'");
    let old = "fn ready() -> bool { true }\n";
    assert_eq!(
        fixture
            .run(old, &old.replace("true", "false"), "edit")
            .decision,
        Decision::Pass
    );
    assert_eq!(
        fixture
            .run(old, &old.replace("ready", "done"), "edit")
            .decision,
        Decision::Block
    );
    assert_eq!(
        fixture
            .run("fn ready() -> i32 { 1 }\n", old, "edit")
            .decision,
        Decision::Block
    );
    assert_eq!(fixture.run(old, old, "write").decision, Decision::Block);
    assert_eq!(
        fixture
            .run(
                "fn f() { let ready = true; }",
                "fn f() { let ready = false; }",
                "edit"
            )
            .decision,
        Decision::Block
    );
}

#[test]
fn empty_comments_block_without_semantic_shortcuts() {
    let fixture = Fixture::new("'comment-shape'");
    for text in ["//\nfn f() {}", "/* */\nfn f() {}", "///  \nfn f() {}"] {
        let result = fixture.run("", text, "write");
        assert_eq!(result.decision, Decision::Block, "{text}");
        assert!(result.judgments.is_empty());
    }
    for text in [
        "// TODO: later\nfn f() {}",
        "/// description\npub fn f() {}",
        "// key = true\nfn f() {}",
        "// /* */\nfn f() {}",
    ] {
        assert_eq!(
            fixture.run("", text, "write").decision,
            Decision::NeedsJudgment
        );
    }
}

#[test]
fn privacy_only_changed_lines_and_safe_diagnostics() {
    let fixture = Fixture::new("'privacy'");
    let private = ["10", "31", "29", "7"].join(".");
    let old = format!("{private}\nold\n");
    assert_eq!(
        fixture
            .run(&old, &format!("{private}\nnew\n"), "edit")
            .decision,
        Decision::Pass
    );
    assert_eq!(fixture.run(&old, "new\n", "edit").decision, Decision::Pass);
    let result = fixture.run("", &old, "write");
    assert_eq!(result.decision, Decision::Block);
    assert_eq!(result.diagnostics[0].line, 1);
    assert!(!serde_json::to_string(&result).unwrap().contains(&private));
}
#[test]
fn whole_comments_context_and_fallback() {
    let fixture = Fixture::new("'comment-length','comment-shape'");
    let old = "// invariant\npub fn f() {}\n";
    let same = fixture.run(old, old, "edit");
    assert_eq!(same.decision, Decision::Pass);
    let changed = fixture.run(old, "// invariant\nfn f() {}\n", "edit");
    assert_eq!(changed.decision, Decision::NeedsJudgment);
    assert_eq!(changed.judgments[0].input.code_context, "fn f() {}");
    assert_eq!(changed.judgments[0].line, 1);
    let long = "// a\n// b\n// c\n// d\nfn {";
    assert_eq!(fixture.run("", long, "write").decision, Decision::Block);
    let doc = "/// a\n/// b\n/// c\n/// d\npub fn f() {}";
    assert_eq!(
        fixture.run("", doc, "write").decision,
        Decision::NeedsJudgment
    );
    let joined = fixture.run(
        "// a\n// b\n\n// c\n// d\n",
        "// a\n// b\n// c\n// d\n",
        "edit",
    );
    assert_eq!(joined.decision, Decision::Block);
}
#[test]
fn unterminated_block_comments_use_actual_lines_at_end_of_file() {
    for language in ["rs", "ts"] {
        for text in ["/* a\nfn main(){}", "/* a\nb\nc"] {
            for suffix in ["", "\n"] {
                let fixture = Fixture::new("'comment-length','comment-shape'");
                let candidate = format!("{text}{suffix}");
                let result = fixture.run_path(&format!("x.{language}"), "", &candidate, "write");
                assert_eq!(
                    result.decision,
                    Decision::NeedsJudgment,
                    "{language}: {candidate:?}"
                );
                assert!(result.diagnostics.is_empty(), "{result:?}");
                assert_eq!(result.judgments.len(), 1);
                let judgment = &result.judgments[0];
                assert_eq!(judgment.line, 1);
                assert_eq!(judgment.input.language, language);
                assert_eq!(judgment.input.comment, text);
                assert!(judgment.input.code_context.is_empty());
            }
        }
    }
}

#[test]
fn unterminated_block_comments_over_three_actual_lines_still_block() {
    for language in ["rs", "ts"] {
        for suffix in ["", "\n"] {
            let fixture = Fixture::new("'comment-length','comment-shape'");
            let candidate = format!("/* a\nb\nc\nd{suffix}");
            let result = fixture.run_path(&format!("x.{language}"), "", &candidate, "write");
            assert_eq!(
                result.decision,
                Decision::Block,
                "{language}: {candidate:?}"
            );
            assert!(result.judgments.is_empty());
            assert_eq!(result.diagnostics.len(), 1, "{result:?}");
            assert_eq!(result.diagnostics[0].rule, Rule::CommentLength.into());
            assert_eq!(result.diagnostics[0].line, 1);
        }
    }
}

#[test]
fn terminated_block_and_line_comments_preserve_text_and_context() {
    for language in ["rs", "ts"] {
        for (text, comment, context) in [
            ("/* a\nb\n*/", "/* a\nb\n*/", ""),
            ("// a", "// a", ""),
            ("// a\nwork();", "// a", "work();"),
        ] {
            for suffix in ["", "\n"] {
                let fixture = Fixture::new("'comment-length','comment-shape'");
                let candidate = format!("{text}{suffix}");
                let result = fixture.run_path(&format!("x.{language}"), "", &candidate, "write");
                assert_eq!(
                    result.decision,
                    Decision::NeedsJudgment,
                    "{language}: {candidate:?}"
                );
                assert!(result.diagnostics.is_empty(), "{result:?}");
                assert_eq!(result.judgments.len(), 1);
                let judgment = &result.judgments[0];
                assert_eq!(judgment.line, 1);
                assert_eq!(judgment.input.language, language);
                assert_eq!(judgment.input.comment, comment);
                assert_eq!(judgment.input.code_context, context);
            }
        }
    }
}

#[test]
fn inserted_comment_context_uses_candidate_code_before_unchanged_suffix() {
    let fixture = Fixture::new("'comment-shape'");
    let suffix = "pub fn retained() {}\n";
    let inserted = "/// Routes each record to its owning shard.\n/// Returns the selected shard; rejects unknown keys.\npub fn route(key: u64) -> Result<u32, RouteError> {\n    lookup(key)\n}\n";
    for (prefix, line) in [("", 1), ("use crate::routing::*;\n\n", 3)] {
        let old = format!("{prefix}{suffix}");
        let candidate = format!("{prefix}{inserted}{suffix}");
        let result = fixture.run(&old, &candidate, "edit");
        assert_eq!(result.decision, Decision::NeedsJudgment);
        assert_eq!(result.judgments.len(), 1);
        assert_eq!(result.judgments[0].line, line);
        assert_eq!(
            result.judgments[0].input.code_context,
            "pub fn route(key: u64) -> Result<u32, RouteError> {\n    lookup(key)\n}"
        );
    }
}

#[test]
fn missing_required_documents_and_present_bad_identifiers_fail_closed() {
    let mut fixture = Fixture::new("'comment-shape','boolean-name','privacy'");
    std::fs::remove_file(&fixture.options.rule_document).unwrap();
    assert_eq!(
        fixture.run("", "// comment", "write").decision,
        Decision::Error
    );
    std::fs::write(&fixture.options.rule_document, "rules").unwrap();
    fixture.options.identifiers = Some(fixture.root.join("absent"));
    assert_eq!(fixture.run("", "", "write").decision, Decision::Error);
}
#[test]
fn oversized_judgment_payload_fails_closed_before_growth() {
    let fixture = Fixture::new("'comment-shape'");
    std::fs::write(&fixture.options.rule_document, "r".repeat(200_000)).unwrap();
    let response = fixture.run("", &"// comment\nfn f() {}\n".repeat(50), "write");
    assert_eq!(response.decision, Decision::Error);
    assert!(response.diagnostics[0].reason.contains("output limit"));
}
#[test]
fn effective_budget_and_enabled_ids_are_authoritative() {
    let fixture = Fixture::new("'boolean-name'");
    std::fs::write(
        fixture.root.join(".edit-time.toml"),
        "version=1\nrules=[]\ntotal_ms=42\n",
    )
    .unwrap();
    let result = fixture.run("", "fn f() { let ready = true; }", "write");
    assert_eq!(result.decision, Decision::Error);
    assert!(result.diagnostics[0]
        .reason
        .contains("cannot remove inherited rules"));
    std::fs::write(
        fixture.root.join(".edit-time.toml"),
        "version=1\nrules=['boolean-name']\ntotal_ms=42\n",
    )
    .unwrap();
    let result = fixture.run("", "fn f() { let ready = true; }", "write");
    assert_eq!(result.decision, Decision::Block);
    assert_eq!(result.diagnostics[0].rule, Rule::BooleanName.into());
    assert_eq!(result.budget_ms, 42);
    let result = fixture.run("", "fn f() { let is_ready = true; }", "write");
    assert_eq!(result.decision, Decision::Pass);
    assert_eq!(result.budget_ms, 42);
}
