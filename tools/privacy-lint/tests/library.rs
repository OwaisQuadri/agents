#[test]
fn shared_scanner_preserves_lines_and_reports_identifier_read_errors() {
    let line = format!("bind {}", std::net::Ipv4Addr::new(10, 83, 27, 19));
    let hits = privacy_lint::scan_line(&line, 9, &[]);
    assert_eq!(hits[0].line_number, 9);
    assert_eq!(hits[0].rule, "rfc 1918 address");
    assert!(privacy_lint::read_local_names(std::path::Path::new("/dev/null/not-a-file")).is_err());
}
