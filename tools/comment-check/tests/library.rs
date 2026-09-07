#[test]
fn typed_spans_preserve_fallback_and_doc_exemption() {
    for extension in ["rs", "ts", "swift", "mm", "pl"] {
        let marker = if extension == "pl" { "#" } else { "//" };
        let text = format!("{marker} a\n{marker} b\n{marker} c\n{marker} d\n");
        let spans = comment_check::extract(&format!("test.{extension}"), &text).unwrap();
        assert_eq!(spans[0].start_line, 1);
        assert_eq!(spans[0].end_line, 4);
        assert_eq!(spans[0].kind, comment_check::CommentKind::Plain);
    }
    let spans =
        comment_check::extract("test.rs", "/// a\n/// b\n/// c\n/// d\nfn f() {}\n").unwrap();
    assert_eq!(spans[0].kind, comment_check::CommentKind::Doc);
    assert!(comment_check::extract("test.txt", "plain").is_none());
}
