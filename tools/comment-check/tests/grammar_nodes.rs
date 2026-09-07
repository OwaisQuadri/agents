#[test]
fn declaration_nodes_have_known_field_names() {
    for (path, text) in [
        ("x.rs", "struct S { ready: bool } fn ready(arg: bool) -> bool { let done = true; arg == done }"),
        ("x.ts", "const ready = true; class S { ready: boolean; readyFn(arg: boolean): boolean { return !arg; } }"),
        ("x.py", "ready = True\ndef done(arg: bool) -> bool:\n    return not arg\n"),
        ("x.go", "package p\ntype S struct { ready bool }; var ready bool; func done(arg bool) bool { local := true; return local == arg }"),
        ("x.java", "class S { boolean ready; boolean done(boolean arg) { boolean local = true; return !arg; } }"),
        ("x.cs", "class S { bool Ready { get; set; } bool Done(bool arg) { bool local = true; return !arg; } }"),
        ("x.cpp", "struct S { bool ready; bool done(bool arg) { auto local = true; return !arg; } };"),
        ("x.kt", "val ready: Boolean = true\nfun done(arg: Boolean): Boolean = !arg\n"),
        ("x.scala", "object S { val ready: Boolean = true; def done(arg: Boolean): Boolean = !arg }"),
        ("x.zig", "const ready: bool = true; fn done(arg: bool) bool { const local = true; return !arg; }"),
    ] {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&comment_check::language(path, text).unwrap()).unwrap();
        let tree = parser.parse(text, None).unwrap();
        assert!(!tree.root_node().has_error(), "{path}: {}", tree.root_node().to_sexp());
        println!("{path}: {}", tree.root_node().to_sexp());
    }
}
