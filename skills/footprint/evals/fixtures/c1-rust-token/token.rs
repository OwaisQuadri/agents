use std::mem::{align_of, size_of};

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u8)]
enum TokenKind {
    Word,
    Keyword,
}

#[derive(Clone, Copy, Debug)]
struct Token {
    kind: TokenKind,
    is_keyword: bool,
    start: u64,
    end: u64,
    line: u64,
    column: u64,
}

fn main() {
    let token = Token { kind: TokenKind::Keyword, is_keyword: true, start: 4, end: 7, line: 1, column: 5 };
    assert_eq!(token.end - token.start, 3);
    println!("instances=10000000 size={} align={}", size_of::<Token>(), align_of::<Token>());
}
