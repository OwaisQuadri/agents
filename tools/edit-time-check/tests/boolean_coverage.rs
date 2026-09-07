use edit_time_check::boolean_findings;
#[test]
fn known_answer_declaration_forms() {
    let mut failures = Vec::new();
    for (path, text, count) in [
        ("x.rs", "struct S { ready: bool } fn done(arg: bool) -> bool { let local = true; arg == local }", 4),
        ("x.ts", "const ready = true; class S { ready: boolean; done(arg: boolean): boolean { return !arg; } }", 4),
        ("x.py", "ready = True\ndef done(arg: bool) -> bool:\n    return not arg\n", 3),
        ("x.go", "package p\ntype S struct { ready bool }; var ready bool; func done(arg bool) bool { local := true; return local == arg }", 5),
        ("x.java", "class S { boolean ready; boolean done(boolean arg) { boolean local = true; return !arg; } }", 4),
        ("x.cs", "class S { bool Ready { get; set; } bool Done(bool arg) { bool local = true; return !arg; } }", 4),
        ("x.cpp", "struct S { bool ready; bool done(bool arg) { auto local = true; return !arg; } };", 4),
        ("x.kt", "val ready: Boolean = true\nfun done(arg: Boolean): Boolean = !arg\n", 3),
        ("x.scala", "object S { val ready: Boolean = true; def done(arg: Boolean): Boolean = !arg }", 3),
        ("x.zig", "const ready: bool = true; fn done(arg: bool) bool { const local = true; return !arg; }", 4),
    ] { let actual = boolean_findings(path, text).unwrap().len(); if actual != count { failures.push(format!("{path}: expected {count}, actual {actual}")); } }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn function_findings_separate_names_from_return_type_evidence() {
    for (path, text, expected_type) in [
        (
            "x.ts",
            "function ready(): boolean { return true; }",
            ": boolean",
        ),
        ("x.rs", "fn ready() -> bool { true }", "bool"),
        ("x.py", "def ready() -> bool:\n    return True\n", "bool"),
    ] {
        let findings = boolean_findings(path, text).unwrap();
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(&text[finding.start_byte..finding.end_byte], "ready");
        assert_eq!(
            &text[finding.evidence.start_byte..finding.evidence.end_byte],
            expected_type
        );
    }
}

#[test]
fn no_truthiness_overloading_or_shadowed_type_inference() {
    for (path, text) in [
        ("x.rs", "fn f() { let ready = !number; }"),
        ("x.cpp", "void f() { auto ready = custom == other; }"),
        (
            "x.ts",
            "let ready = value && other; let done = remote.ready;",
        ),
        (
            "x.py",
            "bool = int\nready: bool = 1\nvalue = left == right\n",
        ),
        ("x.py", "value = left or right\n"),
        ("x.ts", "let isReady: boolean = true;"),
    ] {
        assert!(
            boolean_findings(path, text).unwrap().is_empty(),
            "{path}: {text}"
        );
    }
}
#[test]
fn standard_trait_ownership_requires_resolution() {
    for prefix in [
        "",
        "use std::cmp::PartialEq;",
        "use core::cmp::{PartialEq};",
        "use std::cmp::PartialEq as Equality;",
    ] {
        let name = if prefix.contains("Equality") {
            "Equality"
        } else {
            "PartialEq"
        };
        let text = format!(
            "{prefix} impl {name} for S {{ fn eq(&self, other: &Self) -> bool {{ true }} }}"
        );
        assert!(
            boolean_findings("x.rs", &text).unwrap().is_empty(),
            "{text}"
        );
    }
    for name in [
        "std::cmp::PartialEq",
        "core::cmp::PartialEq",
        "::std::cmp::PartialEq",
    ] {
        let text = format!("impl {name} for S {{ fn eq(&self, other: &Self) -> bool {{ true }} }}");
        assert!(
            boolean_findings("x.rs", &text).unwrap().is_empty(),
            "{text}"
        );
    }
    for prefix in [
        "trait PartialEq {}",
        "#![no_implicit_prelude]",
        "use remote::PartialEq;",
        "use remote::*;",
        "type PartialEq = Remote;",
    ] {
        let text = format!(
            "{prefix} impl PartialEq for S {{ fn eq(&self, other: &Self) -> bool {{ true }} }}"
        );
        assert_eq!(boolean_findings("x.rs", &text).unwrap().len(), 1, "{text}");
    }
    for (prefix, name) in [
        ("mod std {}", "std::cmp::PartialEq"),
        ("", "Unknown"),
        (
            "use std::cmp::PartialEq as Equality; trait Equality {}",
            "Equality",
        ),
    ] {
        let text = format!(
            "{prefix} impl {name} for S {{ fn eq(&self, other: &Self) -> bool {{ true }} }}"
        );
        assert_eq!(boolean_findings("x.rs", &text).unwrap().len(), 1, "{text}");
    }
    for text in [
        "#[cfg(any())] use std::cmp::PartialEq as Equality; impl Equality for S { fn eq(&self, other: &Self) -> bool { true } }",
        "#[no_implicit_prelude] mod m { impl PartialEq for S { fn eq(&self, other: &Self) -> bool { true } } }",
        "mod m { #![no_implicit_prelude] mod nested { impl PartialEq for S { fn eq(&self, other: &Self) -> bool { true } } } }",
        "#![no_std] impl std::cmp::PartialEq for S { fn eq(&self, other: &Self) -> bool { true } }",
        "impl<PartialEq> PartialEq for S { fn eq(&self, other: &Self) -> bool { true } }",
    ] {
        assert_eq!(boolean_findings("x.rs", text).unwrap().len(), 1, "{text}");
    }
    let text = "use std::cmp as comparison; impl comparison::PartialOrd for S { fn lt(&self, other: &Self) -> bool { true } }";
    assert!(boolean_findings("x.rs", text).unwrap().is_empty());
    let text = "use core::cmp::{self as comparison}; impl comparison::PartialOrd for S { fn lt(&self, other: &Self) -> bool { true } }";
    assert!(boolean_findings("x.rs", text).unwrap().is_empty());
}

#[test]
fn external_name_markers_are_not_blanket_exemptions() {
    for (text, count) in [
        ("impl Remote for S { fn ready(&self) -> bool { true } }", 1),
        ("impl ::core::cmp::PartialEq for S { fn eq(&self, other: &Self) -> bool { let local = true; local } }", 1),
        ("#[export_name = \"remote_ready\"] pub fn ready() -> bool { true }", 1),
    ] { assert_eq!(boolean_findings("x.rs", text).unwrap().len(), count, "{text}"); }
}
