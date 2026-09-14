//! Flags over-verbose code comments per docs/comment-style.md's closed whitelist.
//!
//! Whitelist-shape classification ("is this an inexpressible architectural invariant")
//! is judgment, not mechanics, so this first pass enforces the mechanical proxy the
//! owner's review actually objected to: raw verbosity. A non-doc comment block longer
//! than the budget is flagged; doc comments (`///`, `//!`, `/**`) are exempt because
//! the docstring shape is whitelisted and docs/docstring-style.md owns their form.
//!
//! Three modes:
//!   comment-check <file>...             ste-check shape: print FAIL lines, exit nonzero
//!   comment-check                       reads a hook payload from stdin and returns JSON
//!   comment-check --list-json --lang X  reads stdin as source text, prints every
//!                                       comment span (doc and non-doc) as a JSON
//!                                       array with its own text. Whitelist-shape and
//!                                       docstring-position judgment happen downstream
//!                                       of this: this mode is extraction only, same
//!                                       zero-judgment posture as the other two.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

pub const MAX_COMMENT_LINES: usize = 3;

const SLASH_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "go", "swift", "c", "h", "cc", "cpp", "hpp",
    "java", "kt", "kts", "cs", "zig", "scala", "m", "mm",
];

const HASH_EXTENSIONS: &[&str] = &["py", "sh", "zsh", "bash", "rb", "pl", "toml", "yaml", "yml"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LexicalMode {
    Slash { is_rust: bool },
    Hash { is_python: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Grammar {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Go,
    C,
    Cpp,
    Java,
    Kotlin,
    CSharp,
    Zig,
    Scala,
    ObjectiveC,
    Python,
    Bash,
    Zsh,
    Ruby,
    Toml,
    Yaml,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LanguageSpec {
    grammar: Option<Grammar>,
    lexical: LexicalMode,
}

struct CommentExtractor {
    parser: tree_sitter::Parser,
}

#[derive(Debug, Eq, PartialEq)]
enum ParseFailure {
    LanguageVersion,
    NoTree,
    SyntaxError,
}

const VALUE_OPTIONS: &[&str] = &[
    "-c",
    "-C",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--exec-path",
    "--config-env",
    "-R",
    "--repo",
];

const USAGE: &str = "usage: comment-check [file]...  |  comment-check --list-json --lang EXT  (no arguments: PreToolUse hook payload on stdin)";

pub fn run_cli() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--list-json") {
        return run_list_json(&args);
    }
    if args.is_empty() {
        run_hook();
        return ExitCode::SUCCESS;
    }
    let mut extractor = CommentExtractor::new();
    let mut is_failing = false;
    for path in &args {
        for violation in check_path(Path::new(path), &mut extractor) {
            is_failing = true;
            println!("FAIL  {violation}");
        }
    }
    if is_failing {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// `--list-json --lang EXT`: reads source text from stdin, prints a JSON array of every
/// comment span (doc and non-doc) with its own text. Extraction only — no verbosity or
/// shape judgment — mirroring the other two modes' own zero-judgment posture.
fn run_list_json(args: &[String]) -> ExitCode {
    let Some(lang_index) = args.iter().position(|a| a == "--lang") else {
        eprintln!("--list-json requires --lang EXT\n{USAGE}");
        return ExitCode::FAILURE;
    };
    let Some(ext) = args.get(lang_index + 1) else {
        eprintln!("--lang requires a value\n{USAGE}");
        return ExitCode::FAILURE;
    };
    let Some(lang) = lang_for_extension(&format!("x.{ext}")) else {
        eprintln!("unrecognized --lang extension: {ext}");
        return ExitCode::FAILURE;
    };
    let mut text = String::new();
    if std::io::stdin().read_to_string(&mut text).is_err() {
        eprintln!("failed to read stdin");
        return ExitCode::FAILURE;
    }
    let mut extractor = CommentExtractor::new();
    println!("{}", spans_to_json(&text, lang, &mut extractor));
    ExitCode::SUCCESS
}

fn run_hook() {
    let mut raw = String::new();
    if std::io::stdin().read_to_string(&mut raw).is_err() {
        return;
    }
    let Some(command) = extract_command(&raw) else {
        return;
    };
    if !is_git_commit(&command) {
        return;
    }
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    let Some(repo_root) = git_toplevel(&cwd) else {
        return;
    };
    let Some(staged) = staged_files(&repo_root) else {
        return;
    };

    let mut extractor = CommentExtractor::new();
    let mut violations = Vec::new();
    for path in staged {
        if !is_source_path(&path) {
            continue;
        }
        violations.extend(check_path(&repo_root.join(&path), &mut extractor));
    }
    if violations.is_empty() {
        return;
    }

    let mut reason = String::from(
        "Blocked: a staged source file carries an over-verbose comment block.\n\n\
         Standing rule: docs/comment-style.md — a comment ships only in a whitelisted \
         shape, fewer comments beat more, zero is the default target. A non-doc comment \
         block over the length budget almost never fits a whitelisted shape; delete it \
         and make the code explain itself, or shrink it to the invariant it protects.\n",
    );
    for violation in &violations {
        reason.push_str(&format!("\n{violation}"));
    }

    print!(
        "{{\"hookSpecificOutput\":{{\"hookEventName\":\"PreToolUse\",\
         \"permissionDecision\":\"deny\",\"permissionDecisionReason\":{}}},\
         \"systemMessage\":{}}}",
        json_quote(&reason),
        json_quote("Blocked a commit: a staged file carries an over-verbose comment block.")
    );
}

fn is_source_path(path: &str) -> bool {
    lang_for_extension(path).is_some()
}

fn lang_for_extension(path: &str) -> Option<LanguageSpec> {
    let (_, ext) = path.rsplit_once('.')?;
    let grammar = match ext {
        "rs" => Some(Grammar::Rust),
        "ts" => Some(Grammar::TypeScript),
        "tsx" => Some(Grammar::Tsx),
        "js" | "jsx" | "mjs" | "cjs" => Some(Grammar::JavaScript),
        "go" => Some(Grammar::Go),
        "swift" => None,
        "c" => Some(Grammar::C),
        "h" | "cc" | "cpp" | "hpp" => Some(Grammar::Cpp),
        "java" => Some(Grammar::Java),
        "kt" | "kts" => Some(Grammar::Kotlin),
        "cs" => Some(Grammar::CSharp),
        "zig" => Some(Grammar::Zig),
        "scala" => Some(Grammar::Scala),
        "m" => Some(Grammar::ObjectiveC),
        "mm" => None,
        "py" => Some(Grammar::Python),
        "sh" | "bash" => Some(Grammar::Bash),
        "zsh" => Some(Grammar::Zsh),
        "rb" => Some(Grammar::Ruby),
        "pl" => None,
        "toml" => Some(Grammar::Toml),
        "yaml" | "yml" => Some(Grammar::Yaml),
        _ => return None,
    };
    let lexical = if SLASH_EXTENSIONS.contains(&ext) {
        LexicalMode::Slash {
            is_rust: ext == "rs",
        }
    } else if HASH_EXTENSIONS.contains(&ext) {
        LexicalMode::Hash {
            is_python: ext == "py",
        }
    } else {
        return None;
    };
    Some(LanguageSpec { grammar, lexical })
}

fn split_env_words(input: &str) -> Option<std::collections::VecDeque<String>> {
    let mut words = std::collections::VecDeque::new();
    let mut word = String::new();
    let mut quote = None;
    let mut is_word_started = false;
    let mut characters = input.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            let escaped = characters.next()?;
            if quote == Some('\'') && !matches!(escaped, '\'' | '\\') {
                word.push(character);
                word.push(escaped);
            } else {
                match escaped {
                    'c' if quote != Some('"') => break,
                    'f' => word.push('\u{000c}'),
                    'n' => word.push('\n'),
                    'r' => word.push('\r'),
                    't' => word.push('\t'),
                    'v' => word.push('\u{000b}'),
                    '#' | '$' | '"' | '\'' | '\\' => word.push(escaped),
                    '_' if quote == Some('"') => word.push(' '),
                    '_' => {
                        if is_word_started {
                            words.push_back(std::mem::take(&mut word));
                            is_word_started = false;
                        }
                        continue;
                    }
                    _ => return None,
                }
            }
            is_word_started = true;
        } else if let Some(mark) = quote {
            if character == mark {
                quote = None;
            } else {
                word.push(character);
            }
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
            is_word_started = true;
        } else if matches!(character, ' ' | '\t') {
            if is_word_started {
                words.push_back(std::mem::take(&mut word));
                is_word_started = false;
            }
        } else if character == '#' && !is_word_started {
            break;
        } else {
            word.push(character);
            is_word_started = true;
        }
    }
    if quote.is_some() {
        return None;
    }
    if is_word_started {
        words.push_back(word);
    }
    Some(words)
}

fn executable_name(word: &str) -> Option<String> {
    Path::new(word).file_name()?.to_str().map(str::to_owned)
}

fn is_env_assignment(word: &str) -> bool {
    word.split_once('=')
        .is_some_and(|(name, _)| !name.is_empty())
}

fn is_valid_env_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('=')
}

fn consume_env_short_options(
    word: &str,
    words: &mut std::collections::VecDeque<String>,
) -> Option<()> {
    let mut options = word.strip_prefix('-')?.chars();
    while let Some(option) = options.next() {
        match option {
            'i' | 'v' => {}
            'u' | 'C' | 'P' => {
                let attached: String = options.collect();
                let operand = if attached.is_empty() {
                    words.pop_front()?
                } else {
                    attached
                };
                if (option == 'u' && !is_valid_env_name(&operand))
                    || (matches!(option, 'C' | 'P') && operand.is_empty())
                {
                    return None;
                }
                return Some(());
            }
            'S' => {
                let attached: String = options.collect();
                let split = if attached.is_empty() {
                    words.pop_front()?
                } else {
                    attached
                };
                let mut split_words = split_env_words(&split)?;
                split_words.append(words);
                *words = split_words;
                return Some(());
            }
            _ => return None,
        }
    }
    Some(())
}

fn shebang_interpreter(line: &str) -> Option<String> {
    let directive = line.strip_prefix("#!")?.trim_start_matches([' ', '\t']);
    let command_end = directive.find([' ', '\t']).unwrap_or(directive.len());
    let command = &directive[..command_end];
    if !command.starts_with('/') {
        return None;
    }
    let name = executable_name(command)?;
    if name != "env" {
        return Some(name);
    }

    let arguments = directive[command_end..].trim_start_matches([' ', '\t']);
    let mut words = if let Some(split) = arguments.strip_prefix("-S") {
        split_env_words(split.trim_start())?
    } else if let Some(split) = arguments.strip_prefix("--split-string=") {
        split_env_words(split)?
    } else if arguments == "--split-string" {
        std::collections::VecDeque::new()
    } else if let Some(split) = arguments
        .strip_prefix("--split-string")
        .filter(|split| matches!(split.as_bytes().first(), Some(b' ' | b'\t')))
    {
        split_env_words(split.trim_start())?
    } else {
        arguments
            .split_ascii_whitespace()
            .map(str::to_owned)
            .collect()
    };
    let mut is_parsing_options = true;
    while let Some(word) = words.pop_front() {
        if is_parsing_options {
            if word.starts_with('-') && !word.starts_with("--") {
                consume_env_short_options(&word, &mut words)?;
                continue;
            }
            match word.as_str() {
                "--" => {
                    is_parsing_options = false;
                    continue;
                }
                "--unset" => {
                    let name = words.pop_front()?;
                    if !is_valid_env_name(&name) {
                        return None;
                    }
                    continue;
                }
                "--chdir" => {
                    if words.pop_front()?.is_empty() {
                        return None;
                    }
                    continue;
                }
                "--ignore-environment" => continue,
                _ if word.starts_with("--unset=") => {
                    if !is_valid_env_name(word.strip_prefix("--unset=")?) {
                        return None;
                    }
                    continue;
                }
                _ if word.starts_with("--chdir=") => {
                    if word.strip_prefix("--chdir=")?.is_empty() {
                        return None;
                    }
                    continue;
                }
                _ if word.starts_with('-') => return None,
                _ => {}
            }
        }
        if word.contains('=') {
            if !is_env_assignment(&word) {
                return None;
            }
            is_parsing_options = false;
            continue;
        }
        if executable_name(&word).as_deref() == Some("uv")
            && words.front().is_some_and(|next| next == "run")
        {
            words.pop_front();
            return executable_name(&words.pop_front()?);
        }
        return executable_name(&word);
    }
    None
}

fn is_versioned_interpreter(name: &str, base: &str) -> bool {
    name == base
        || name.strip_prefix(base).is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix.split('.').all(|part| {
                    !part.is_empty() && part.chars().all(|character| character.is_ascii_digit())
                })
        })
}

const BASH_INTERPRETERS: &[&str] = &["sh", "bash", "dash", "ksh", "ash", "mksh", "yash", "posh"];
const HASH_INTERPRETERS: &[&str] = &["fish", "csh", "tcsh", "tclsh"];
const JAVASCRIPT_INTERPRETERS: &[&str] = &["node", "deno", "bun"];

fn is_interpreter_family(name: &str, bases: &[&str]) -> bool {
    bases
        .iter()
        .any(|base| is_versioned_interpreter(name, base))
}

fn lang_for_shebang(text: &str) -> Option<LanguageSpec> {
    let name = shebang_interpreter(text.lines().next()?)?;
    if is_versioned_interpreter(&name, "python") {
        return Some(LanguageSpec {
            grammar: Some(Grammar::Python),
            lexical: LexicalMode::Hash { is_python: true },
        });
    }
    let spec = if is_versioned_interpreter(&name, "zsh") {
        LanguageSpec {
            grammar: Some(Grammar::Zsh),
            lexical: LexicalMode::Hash { is_python: false },
        }
    } else if is_interpreter_family(&name, BASH_INTERPRETERS) {
        LanguageSpec {
            grammar: Some(Grammar::Bash),
            lexical: LexicalMode::Hash { is_python: false },
        }
    } else if is_interpreter_family(&name, HASH_INTERPRETERS) {
        LanguageSpec {
            grammar: None,
            lexical: LexicalMode::Hash { is_python: false },
        }
    } else if is_interpreter_family(&name, JAVASCRIPT_INTERPRETERS) {
        LanguageSpec {
            grammar: Some(Grammar::JavaScript),
            lexical: LexicalMode::Slash { is_rust: false },
        }
    } else if is_versioned_interpreter(&name, "ruby") {
        LanguageSpec {
            grammar: Some(Grammar::Ruby),
            lexical: LexicalMode::Hash { is_python: false },
        }
    } else if is_versioned_interpreter(&name, "perl") {
        LanguageSpec {
            grammar: None,
            lexical: LexicalMode::Hash { is_python: false },
        }
    } else {
        return None;
    };
    Some(spec)
}

fn is_interpreter_directive(line: &str) -> bool {
    shebang_interpreter(line).is_some()
}

fn check_path(path: &Path, extractor: &mut CommentExtractor) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let lang = match path
        .to_str()
        .and_then(lang_for_extension)
        .or_else(|| lang_for_shebang(&text))
    {
        Some(lang) => lang,
        None => return Vec::new(),
    };
    long_blocks(&text, lang, extractor)
        .into_iter()
        .map(|(start, end)| {
            format!(
                "{}:{start}-{end}  non-doc comment block of {} lines (budget {MAX_COMMENT_LINES})",
                path.display(),
                end - start + 1
            )
        })
        .collect()
}

#[derive(Debug, Eq, PartialEq)]
pub enum CommentKind {
    Doc,
    Plain,
}

#[derive(Debug, Eq, PartialEq)]
pub struct CommentSpan {
    pub start_line: usize,
    pub end_line: usize,
    pub kind: CommentKind,
    pub is_full_line: bool,
}

pub fn extract(path: &str, text: &str) -> Option<Vec<CommentSpan>> {
    let spec = lang_for_extension(path).or_else(|| lang_for_shebang(text))?;
    Some(merged_spans(text, spec, &mut CommentExtractor::new()))
}

pub fn language(path: &str, text: &str) -> Option<tree_sitter::Language> {
    let spec = lang_for_extension(path).or_else(|| lang_for_shebang(text))?;
    spec.grammar.map(grammar_language)
}

/// Reports the language label used in semantic comment judgments.
/// The inputs are the candidate path and complete source text.
/// The output is a stable extension or interpreter-family label.
/// This lookup cannot return an error.
pub fn language_label(path: &str, text: &str) -> String {
    if let Some(extension) = Path::new(path).extension().and_then(|value| value.to_str()) {
        return extension.to_owned();
    }
    let Some(name) = text.lines().next().and_then(shebang_interpreter) else {
        return "text".into();
    };
    for (bases, label) in [
        (&["python"][..], "python"),
        (&["ruby"][..], "ruby"),
        (&["perl"][..], "perl"),
        (JAVASCRIPT_INTERPRETERS, "js"),
        (&["zsh"][..], "zsh"),
        (&["fish"][..], "fish"),
        (&["csh", "tcsh"][..], "csh"),
        (&["tclsh"][..], "tcl"),
        (BASH_INTERPRETERS, "sh"),
    ] {
        if is_interpreter_family(&name, bases) {
            return label.into();
        }
    }
    "text".into()
}

impl CommentExtractor {
    fn new() -> Self {
        Self {
            parser: tree_sitter::Parser::new(),
        }
    }

    fn spans(&mut self, text: &str, spec: LanguageSpec) -> Vec<CommentSpan> {
        match spec.grammar {
            Some(grammar) => self
                .tree_spans(text, grammar, spec.lexical)
                .unwrap_or_else(|_| lexical_spans(text, spec.lexical)),
            None => lexical_spans(text, spec.lexical),
        }
    }

    fn tree_spans(
        &mut self,
        text: &str,
        grammar: Grammar,
        lexical: LexicalMode,
    ) -> Result<Vec<CommentSpan>, ParseFailure> {
        self.parser
            .set_language(&grammar_language(grammar))
            .map_err(|_| ParseFailure::LanguageVersion)?;
        let tree = self.parser.parse(text, None).ok_or(ParseFailure::NoTree)?;
        let root = tree.root_node();
        if root.has_error() {
            return Err(ParseFailure::SyntaxError);
        }
        let bytes = text.as_bytes();
        let mut nodes = Vec::new();
        collect_comment_nodes(root, &mut nodes);
        nodes.sort_unstable_by_key(|node| (node.start_byte(), node.end_byte()));
        nodes.dedup_by_key(|node| (node.start_byte(), node.end_byte()));
        Ok(nodes
            .into_iter()
            .filter_map(|node| {
                let start_byte = node.start_byte();
                let comment = bytes.get(start_byte..node.end_byte())?;
                if !is_comment_start(comment, lexical, grammar) {
                    return None;
                }
                let start_line = node.start_position().row + 1;
                let end = node.end_position();
                let end_line = if end.column == 0 && end.row + 1 > start_line {
                    end.row
                } else {
                    end.row + 1
                };
                Some(CommentSpan {
                    start_line,
                    end_line,
                    kind: classify_comment(comment, lexical, start_line),
                    is_full_line: is_full_line(bytes, start_byte),
                })
            })
            .collect())
    }
}

fn grammar_language(grammar: Grammar) -> tree_sitter::Language {
    match grammar {
        Grammar::Rust => tree_sitter_rust::LANGUAGE.into(),
        Grammar::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Grammar::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Grammar::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Grammar::Go => tree_sitter_go::LANGUAGE.into(),
        Grammar::C => tree_sitter_c::LANGUAGE.into(),
        Grammar::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        Grammar::Java => tree_sitter_java::LANGUAGE.into(),
        Grammar::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
        Grammar::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
        Grammar::Zig => tree_sitter_zig::LANGUAGE.into(),
        Grammar::Scala => tree_sitter_scala::LANGUAGE.into(),
        Grammar::ObjectiveC => tree_sitter_objc::LANGUAGE.into(),
        Grammar::Python => tree_sitter_python::LANGUAGE.into(),
        Grammar::Bash => tree_sitter_bash::LANGUAGE.into(),
        Grammar::Zsh => tree_sitter_zsh::LANGUAGE.into(),
        Grammar::Ruby => tree_sitter_ruby::LANGUAGE.into(),
        Grammar::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
        Grammar::Yaml => tree_sitter_yaml::LANGUAGE.into(),
    }
}

fn collect_comment_nodes<'tree>(
    root: tree_sitter::Node<'tree>,
    nodes: &mut Vec<tree_sitter::Node<'tree>>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind().ends_with("comment") {
            nodes.push(node);
            continue;
        }
        for index in (0..node.child_count()).rev() {
            if let Some(child) = node.child(index) {
                stack.push(child);
            }
        }
    }
}

fn is_comment_start(bytes: &[u8], mode: LexicalMode, grammar: Grammar) -> bool {
    match mode {
        LexicalMode::Slash { .. } => bytes.starts_with(b"//") || bytes.starts_with(b"/*"),
        LexicalMode::Hash { .. } => {
            bytes.starts_with(b"#") || (grammar == Grammar::Ruby && bytes.starts_with(b"=begin"))
        }
    }
}

fn is_full_line(bytes: &[u8], start_byte: usize) -> bool {
    let line_start = bytes[..start_byte]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    std::str::from_utf8(&bytes[line_start..start_byte])
        .is_ok_and(|prefix| prefix.chars().all(char::is_whitespace))
}

fn classify_comment(bytes: &[u8], mode: LexicalMode, start_line: usize) -> CommentKind {
    let is_shebang =
        start_line == 1 && std::str::from_utf8(bytes).is_ok_and(is_interpreter_directive);
    let is_doc = is_shebang
        || match mode {
            LexicalMode::Slash { is_rust } => {
                if bytes.starts_with(b"//") {
                    if is_rust {
                        bytes.starts_with(b"//!")
                            || (bytes.starts_with(b"///") && !bytes.starts_with(b"////"))
                    } else {
                        bytes.starts_with(b"//!") || bytes.starts_with(b"///")
                    }
                } else {
                    bytes.starts_with(b"/*!")
                        || (bytes.starts_with(b"/**")
                            && !bytes.starts_with(b"/***")
                            && !bytes.starts_with(b"/**/"))
                }
            }
            LexicalMode::Hash { .. } => false,
        };
    if is_doc {
        CommentKind::Doc
    } else {
        CommentKind::Plain
    }
}

fn lexical_spans(text: &str, mode: LexicalMode) -> Vec<CommentSpan> {
    match mode {
        LexicalMode::Slash { is_rust } => comment_spans(text, is_rust),
        LexicalMode::Hash { is_python } => hash_comment_spans(text, is_python),
    }
}

/// 1-based (start, end) line spans of every non-doc comment block longer than the
/// budget. Consecutive full-line comments merge into one block; a trailing
/// comment after code never merges with the lines below it.
fn long_blocks(
    text: &str,
    spec: LanguageSpec,
    extractor: &mut CommentExtractor,
) -> Vec<(usize, usize)> {
    merged_spans(text, spec, extractor)
        .into_iter()
        .filter(|b| b.kind == CommentKind::Plain)
        .filter(|b| b.end_line - b.start_line + 1 > MAX_COMMENT_LINES)
        .map(|b| (b.start_line, b.end_line))
        .collect()
}

/// Every comment block (doc and non-doc), consecutive full-line comments merged into
/// one span the same way `long_blocks` merges them — shared so `--list-json` and the
/// length check see identical block boundaries.
fn merged_spans(
    text: &str,
    spec: LanguageSpec,
    extractor: &mut CommentExtractor,
) -> Vec<CommentSpan> {
    let spans = extractor.spans(text, spec);
    let mut blocks: Vec<CommentSpan> = Vec::new();
    for span in spans {
        if let Some(last) = blocks.last_mut() {
            if last.kind == span.kind
                && last.is_full_line
                && span.is_full_line
                && span.start_line == last.end_line + 1
            {
                last.end_line = span.end_line;
                continue;
            }
        }
        blocks.push(span);
    }
    blocks
}

/// Renders every comment block in `text` (doc and non-doc) as a JSON array of
/// `{start_line, end_line, kind, text}`, `text` being the exact source lines (with
/// their own leading comment markers) the span covers.
fn spans_to_json(text: &str, spec: LanguageSpec, extractor: &mut CommentExtractor) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = String::from("[");
    for (i, span) in merged_spans(text, spec, extractor).into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let kind = match span.kind {
            CommentKind::Doc => "doc",
            CommentKind::Plain => "plain",
        };
        let span_text = lines
            .get(span.start_line - 1..span.end_line)
            .map(|slice| slice.join("\n"))
            .unwrap_or_default();
        out.push_str(&format!(
            r#"{{"start_line":{},"end_line":{},"kind":"{kind}","text":{}}}"#,
            span.start_line,
            span.end_line,
            json_quote(&span_text)
        ));
    }
    out.push(']');
    out
}

/// Lexes the source just enough to find comments without being fooled by comment
/// markers inside string literals: plain strings in both languages, raw strings and
/// char/lifetime quotes in Rust, template literals in TypeScript.
fn comment_spans(text: &str, is_rust: bool) -> Vec<CommentSpan> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut i = 0;
    let mut line = 1;
    let mut is_line_blank_so_far = true;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '\n' => {
                line += 1;
                is_line_blank_so_far = true;
                i += 1;
            }
            '/' if next == Some('/') => {
                let third = chars.get(i + 2).copied();
                let fourth = chars.get(i + 3).copied();
                let is_doc = if is_rust {
                    third == Some('!') || (third == Some('/') && fourth != Some('/'))
                } else {
                    third == Some('!') || third == Some('/')
                };
                let is_full_line = is_line_blank_so_far;
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                spans.push(CommentSpan {
                    start_line: line,
                    end_line: line,
                    kind: if is_doc {
                        CommentKind::Doc
                    } else {
                        CommentKind::Plain
                    },
                    is_full_line,
                });
            }
            '/' if next == Some('*') => {
                let third = chars.get(i + 2).copied();
                let fourth = chars.get(i + 3).copied();
                let is_doc = third == Some('!')
                    || (third == Some('*') && fourth != Some('*') && fourth != Some('/'));
                let is_full_line = is_line_blank_so_far;
                let start_line = line;
                i += 2;
                let mut depth = 1;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '\n' {
                        line += 1;
                    } else if is_rust && chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 1;
                    } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 1;
                    }
                    i += 1;
                }
                spans.push(CommentSpan {
                    start_line,
                    end_line: line,
                    kind: if is_doc {
                        CommentKind::Doc
                    } else {
                        CommentKind::Plain
                    },
                    is_full_line,
                });
                is_line_blank_so_far = false;
            }
            '"' => {
                is_line_blank_so_far = false;
                i += 1;
                while i < chars.len() {
                    match chars[i] {
                        '"' => {
                            i += 1;
                            break;
                        }
                        '\\' => i += 2,
                        '\n' => {
                            line += 1;
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
            }
            'r' if is_rust && matches!(next, Some('"') | Some('#')) => {
                let mut hashes = 0;
                let mut j = i + 1;
                while chars.get(j) == Some(&'#') {
                    hashes += 1;
                    j += 1;
                }
                if chars.get(j) != Some(&'"') {
                    is_line_blank_so_far = false;
                    i += 1;
                    continue;
                }
                is_line_blank_so_far = false;
                i = j + 1;
                while i < chars.len() {
                    if chars[i] == '\n' {
                        line += 1;
                    } else if chars[i] == '"'
                        && (1..=hashes).all(|k| chars.get(i + k) == Some(&'#'))
                    {
                        i += hashes + 1;
                        break;
                    }
                    i += 1;
                }
            }
            '\'' if is_rust => {
                is_line_blank_so_far = false;
                if next == Some('\\') {
                    i += 2;
                    while i < chars.len() && chars[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                } else if chars.get(i + 2) == Some(&'\'') {
                    i += 3;
                } else {
                    // a lifetime, not a char literal: consume only the quote
                    i += 1;
                }
            }
            '\'' | '`' if !is_rust => {
                let quote = c;
                is_line_blank_so_far = false;
                i += 1;
                while i < chars.len() {
                    match chars[i] {
                        q if q == quote => {
                            i += 1;
                            break;
                        }
                        '\\' => i += 2,
                        '\n' => {
                            line += 1;
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
            }
            _ => {
                if !c.is_whitespace() {
                    is_line_blank_so_far = false;
                }
                i += 1;
            }
        }
    }
    spans
}

/// Lexes hash-comment languages: `#` opens a comment only at line start or after
/// whitespace, so `$#`, `${#x}`, and mid-word hashes stay code. Python's
/// triple-quoted strings and both quote styles are skipped; a shebang is exempt.
fn hash_comment_spans(text: &str, is_python: bool) -> Vec<CommentSpan> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut i = 0;
    let mut line = 1;
    let mut is_line_blank_so_far = true;
    let mut prev_is_boundary = true;
    let is_first_line_shebang = text.lines().next().is_some_and(is_interpreter_directive);
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\n' => {
                line += 1;
                is_line_blank_so_far = true;
                prev_is_boundary = true;
                i += 1;
            }
            '#' if prev_is_boundary => {
                let is_shebang = line == 1 && is_first_line_shebang;
                let is_full_line = is_line_blank_so_far;
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                spans.push(CommentSpan {
                    start_line: line,
                    end_line: line,
                    kind: if is_shebang {
                        CommentKind::Doc
                    } else {
                        CommentKind::Plain
                    },
                    is_full_line,
                });
            }
            '"' | '\'' => {
                let quote = c;
                is_line_blank_so_far = false;
                prev_is_boundary = false;
                let is_triple = is_python
                    && chars.get(i + 1) == Some(&quote)
                    && chars.get(i + 2) == Some(&quote);
                if is_triple {
                    i += 3;
                    while i < chars.len() {
                        if chars[i] == '\n' {
                            line += 1;
                        } else if chars[i] == '\\' {
                            i += 1;
                        } else if chars[i] == quote
                            && chars.get(i + 1) == Some(&quote)
                            && chars.get(i + 2) == Some(&quote)
                        {
                            i += 3;
                            break;
                        }
                        i += 1;
                    }
                } else {
                    i += 1;
                    while i < chars.len() {
                        match chars[i] {
                            q if q == quote => {
                                i += 1;
                                break;
                            }
                            '\\' if quote == '"' || is_python => i += 2,
                            '\n' => {
                                line += 1;
                                i += 1;
                            }
                            _ => i += 1,
                        }
                    }
                }
            }
            _ => {
                if !c.is_whitespace() {
                    is_line_blank_so_far = false;
                }
                prev_is_boundary = c.is_whitespace();
                i += 1;
            }
        }
    }
    spans
}

fn staged_files(repo_root: &Path) -> Option<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("diff")
        .arg("--cached")
        .arg("--name-only")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Some(text.lines().map(str::to_string).collect())
}

fn git_toplevel(cwd: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Some(PathBuf::from(text.trim()))
}

fn is_git_commit(command: &str) -> bool {
    let lowered = command.to_lowercase();
    let tokens: Vec<&str> = lowered
        .split_whitespace()
        .map(|token| token.trim_matches(|c| c == '"' || c == '\''))
        .collect();
    is_verb_sequence(&tokens, &["git", "commit"])
}

fn is_verb_sequence(tokens: &[&str], verbs: &[&str]) -> bool {
    for (start, token) in tokens.iter().enumerate() {
        if *token != verbs[0] {
            continue;
        }
        let mut index = start + 1;
        let mut matched = 1;
        while index < tokens.len() && matched < verbs.len() {
            let token = tokens[index];
            if token.starts_with('-') {
                if VALUE_OPTIONS.contains(&token) {
                    index += 1;
                }
                index += 1;
                continue;
            }
            if token != verbs[matched] {
                break;
            }
            matched += 1;
            index += 1;
        }
        if matched == verbs.len() {
            return true;
        }
    }
    false
}

fn extract_command(payload: &str) -> Option<String> {
    let scope = payload.find("\"tool_input\"").unwrap_or(0);
    let key = payload[scope..].find("\"command\"")? + scope;
    let open = payload[key + "\"command\"".len()..].find('"')? + key + "\"command\"".len();
    decode_json_string(&payload[open + 1..])
}

fn decode_json_string(rest: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'u' => {
                    let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                    let point = u32::from_str_radix(&hex, 16).ok()?;
                    out.push(char::from_u32(point).unwrap_or('\u{fffd}'));
                }
                other => out.push(other),
            },
            other => out.push(other),
        }
    }
    None
}

fn json_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST: LanguageSpec = LanguageSpec {
        grammar: Some(Grammar::Rust),
        lexical: LexicalMode::Slash { is_rust: true },
    };
    const TS: LanguageSpec = LanguageSpec {
        grammar: Some(Grammar::TypeScript),
        lexical: LexicalMode::Slash { is_rust: false },
    };
    const TSX: LanguageSpec = LanguageSpec {
        grammar: Some(Grammar::Tsx),
        lexical: LexicalMode::Slash { is_rust: false },
    };
    const PYTHON: LanguageSpec = LanguageSpec {
        grammar: Some(Grammar::Python),
        lexical: LexicalMode::Hash { is_python: true },
    };
    const SHELL: LanguageSpec = LanguageSpec {
        grammar: Some(Grammar::Bash),
        lexical: LexicalMode::Hash { is_python: false },
    };

    fn blocks(text: &str, spec: LanguageSpec) -> Vec<(usize, usize)> {
        long_blocks(text, spec, &mut CommentExtractor::new())
    }

    fn spans(text: &str, spec: LanguageSpec) -> Vec<CommentSpan> {
        CommentExtractor::new().spans(text, spec)
    }

    fn rust_blocks(text: &str) -> Vec<(usize, usize)> {
        blocks(text, RUST)
    }

    #[test]
    fn short_comment_blocks_pass() {
        let text = "// one\n// two\n// three\nfn main() {}\n";
        assert!(rust_blocks(text).is_empty());
    }

    #[test]
    fn a_four_line_run_of_line_comments_is_flagged() {
        let text = "// one\n// two\n// three\n// four\nfn main() {}\n";
        assert_eq!(rust_blocks(text), vec![(1, 4)]);
    }

    #[test]
    fn doc_comments_are_exempt() {
        let text = "//! a\n//! b\n//! c\n//! d\n//! e\n//! f\n/// g\n/// h\n/// i\n/// j\n/// k\nfn main() {}\n";
        assert!(rust_blocks(text).is_empty());
        let jsdoc = "/**\n * a\n * b\n * c\n * d\n * e\n */\nexport function f() {}\n";
        assert!(blocks(jsdoc, TS).is_empty());
    }

    #[test]
    fn a_long_block_comment_is_flagged() {
        let text = "fn main() {}\n/* a\nb\nc\nd\ne */\n";
        assert_eq!(rust_blocks(text), vec![(2, 6)]);
        assert_eq!(blocks(text, TS), vec![(2, 6)]);
    }

    #[test]
    fn a_blank_line_splits_two_short_runs() {
        let text = "// one\n// two\n// three\n\n// four\n// five\nfn main() {}\n";
        assert!(rust_blocks(text).is_empty());
    }

    #[test]
    fn a_trailing_comment_does_not_merge_with_the_run_below() {
        let text = "let x = 1; // trailing\n// a\n// b\n// c\nfn main() {}\n";
        assert!(rust_blocks(text).is_empty());
    }

    #[test]
    fn comment_markers_inside_strings_are_not_comments() {
        let text = "let a = \"// one\";\nlet b = \"// two\";\nlet c = \"// three\";\nlet d = \"// four\";\nlet e = \"// five\";\n";
        assert!(rust_blocks(text).is_empty());
        let raw = "let a = r#\"// one\n// two\n// three\n// four\n// five\"#;\n";
        assert!(rust_blocks(raw).is_empty());
        let template = "const a = `// one\n// two\n// three\n// four\n// five`;\n";
        assert!(blocks(template, TS).is_empty());
    }

    #[test]
    fn a_lifetime_quote_does_not_swallow_the_rest_of_the_file() {
        let text = "fn f<'a>(x: &'a str) {}\n// a\n// b\n// c\n// d\n// e\n";
        assert_eq!(rust_blocks(text), vec![(2, 6)]);
    }

    #[test]
    fn triple_slash_is_doc_in_every_slash_language() {
        let text = "/// a\n/// b\n/// c\n/// d\n/// e\nexport {}\n";
        assert!(blocks(text, TS).is_empty());
    }

    #[test]
    fn a_four_line_hash_run_is_flagged() {
        let text = "# one\n# two\n# three\n# four\nx = 1\n";
        assert_eq!(blocks(text, PYTHON), vec![(1, 4)]);
        assert!(blocks("# one\n# two\n# three\nx = 1\n", SHELL).is_empty());
    }

    #[test]
    fn hash_inside_strings_and_words_is_not_a_comment() {
        let text = "a = \"# one\"\nb = \"# two\"\nc = \"# three\"\nd = \"# four\"\n";
        assert!(blocks(text, PYTHON).is_empty());
        let doc = "def f():\n    \"\"\"\n    # a\n    # b\n    # c\n    # d\n    \"\"\"\n";
        assert!(blocks(doc, PYTHON).is_empty());
        let shell = "echo $#\nn=${#arr}\ncase x#y in esac\nz=1\n";
        assert!(blocks(shell, SHELL).is_empty());
    }

    #[test]
    fn a_shebang_does_not_merge_with_the_run_below() {
        let text = "#!/bin/zsh\n# a\n# b\n# c\nx=1\n";
        assert!(blocks(text, SHELL).is_empty());
        let text = "#!/bin/zsh\n# a\n# b\n# c\n# d\nx=1\n";
        assert_eq!(blocks(text, SHELL), vec![(2, 5)]);
    }

    #[test]
    fn shell_heredoc_shebangs_are_not_comments() {
        for text in [
            "cat <<EOF\n#!/bin/zsh\nEOF\n",
            "cat <<'EOF'\n#!/bin/zsh\nEOF\n",
        ] {
            assert!(spans(text, SHELL).is_empty());
        }
    }

    #[test]
    fn unsupported_or_later_shell_hash_bangs_are_plain_comments() {
        for (text, line) in [
            ("#! not an interpreter\necho ready\n", 1),
            ("echo ready\n#! not a shebang\n", 2),
        ] {
            assert_eq!(
                spans(text, SHELL),
                vec![CommentSpan {
                    start_line: line,
                    end_line: line,
                    kind: CommentKind::Plain,
                    is_full_line: true,
                }]
            );
        }
    }

    #[test]
    fn source_extensions_span_both_comment_families() {
        assert!(is_source_path("tools/foo/src/main.rs"));
        assert!(is_source_path("pi/extensions/guard.ts"));
        assert!(is_source_path("web/app.tsx"));
        assert!(is_source_path("install.sh"));
        assert!(is_source_path("scripts/run.py"));
        assert!(is_source_path("config/settings.toml"));
        assert!(!is_source_path("docs/comment-style.md"));
        assert!(!is_source_path("README"));
    }

    #[test]
    fn extensionless_files_are_classified_by_shebang() {
        assert_eq!(
            lang_for_shebang("#!/bin/zsh\n# a\n").map(|spec| spec.grammar),
            Some(Some(Grammar::Zsh))
        );
        assert_eq!(
            lang_for_shebang("#!/usr/bin/env python3\n").map(|spec| spec.grammar),
            Some(Some(Grammar::Python))
        );
        assert_eq!(lang_for_shebang("plain text\n"), None);
        assert_eq!(
            extract("script", "#!/usr/bin/fish\n# one\n# two\n# three\n# four\n"),
            Some(vec![
                CommentSpan {
                    start_line: 1,
                    end_line: 1,
                    kind: CommentKind::Doc,
                    is_full_line: true,
                },
                CommentSpan {
                    start_line: 2,
                    end_line: 5,
                    kind: CommentKind::Plain,
                    is_full_line: true,
                },
            ])
        );
        assert_eq!(
            extract(
                "script",
                "#!/usr/bin/env node\nclass A {\n#a = 1;\n#b = 2;\n#c = 3;\n#d = 4;\n}\n"
            ),
            Some(vec![])
        );
        assert_eq!(language_label("script", "#!/usr/bin/env node\n"), "js");
        assert_eq!(language_label("script", "#!/usr/bin/fish\n"), "fish");
        assert_eq!(language_label("script", "#!/usr/bin/pwsh\n"), "text");
        assert_eq!(language_label("script", "#!/bin/tcsh\n"), "csh");
        assert_eq!(language_label("script", "#!/usr/bin/tclsh8.6\n"), "tcl");
        assert_eq!(language_label("script", "#!/usr/bin/ksh93\n"), "sh");
        assert_eq!(language_label("script", "#!/usr/bin/env ruby3.1\n"), "ruby");
        assert_eq!(
            language_label("script", "#!/usr/bin/env perl5.36\n"),
            "perl"
        );
        for directive in [
            "#!/usr/bin/env -S uv run python3\n",
            "#!/usr/bin/env -S /usr/bin/uv run python3\n",
        ] {
            assert_eq!(language_label("script", directive), "python");
        }
    }

    #[test]
    fn commit_gating_matches_the_sibling_checkers() {
        assert!(is_git_commit("git commit -m x"));
        assert!(is_git_commit("git -C ~/repo commit -m x"));
        assert!(is_git_commit("git add -A && git commit -m x"));
        assert!(!is_git_commit("git log --oneline"));
        assert!(!is_git_commit("gh pr create --body x"));
    }

    #[test]
    fn extracts_command_from_pretooluse_payload() {
        let payload = r#"{"tool_name":"Bash","tool_input":{"command":"git commit -m x"}}"#;
        assert_eq!(extract_command(payload).as_deref(), Some("git commit -m x"));
    }

    #[test]
    fn list_json_includes_doc_and_plain_spans_with_their_own_text() {
        let text = "/// docstring\nfn public_fn() {}\n\n// plain comment\n// second line\nfn private_helper() {}\n";
        let json = spans_to_json(text, RUST, &mut CommentExtractor::new());
        assert_eq!(
            json,
            r#"[{"start_line":1,"end_line":1,"kind":"doc","text":"/// docstring"},{"start_line":4,"end_line":5,"kind":"plain","text":"// plain comment\n// second line"}]"#
        );
    }

    #[test]
    fn list_json_never_filters_by_length_unlike_long_blocks() {
        // a run over MAX_COMMENT_LINES: long_blocks flags it, --list-json still lists
        // it verbatim -- length filtering is long_blocks's job alone, list-json is pure
        // extraction for a downstream judge to classify.
        let text = "// one\n// two\n// three\n// four\nfn main() {}\n";
        assert_eq!(rust_blocks(text), vec![(1, 4)]);
        let json = spans_to_json(text, RUST, &mut CommentExtractor::new());
        assert!(json.contains(r#""start_line":1,"end_line":4,"kind":"plain"#));
    }

    #[test]
    fn list_json_escapes_quotes_and_newlines_in_span_text() {
        let text = "// says \"hi\"\nfn main() {}\n";
        let json = spans_to_json(text, RUST, &mut CommentExtractor::new());
        assert_eq!(
            json,
            r#"[{"start_line":1,"end_line":1,"kind":"plain","text":"// says \"hi\""}]"#
        );
    }

    #[test]
    fn list_json_on_empty_input_is_an_empty_array() {
        assert_eq!(spans_to_json("", RUST, &mut CommentExtractor::new()), "[]");
        assert_eq!(
            spans_to_json("fn main() {}\n", RUST, &mut CommentExtractor::new()),
            "[]"
        );
    }

    #[test]
    fn list_json_works_across_hash_and_slash_languages() {
        let py = "# a note\ndef f():\n    pass\n";
        assert_eq!(
            spans_to_json(py, PYTHON, &mut CommentExtractor::new()),
            r##"[{"start_line":1,"end_line":1,"kind":"plain","text":"# a note"}]"##
        );
    }

    #[test]
    fn language_extensions_select_their_grammar() {
        assert_eq!(
            lang_for_extension("source.ts").map(|spec| spec.grammar),
            Some(Some(Grammar::TypeScript))
        );
        assert_eq!(
            lang_for_extension("source.tsx").map(|spec| spec.grammar),
            Some(Some(Grammar::Tsx))
        );
        assert_eq!(
            lang_for_extension("source.js").map(|spec| spec.grammar),
            Some(Some(Grammar::JavaScript))
        );
        assert_eq!(
            lang_for_shebang("#!/usr/bin/env python3\n").map(|spec| spec.grammar),
            Some(Some(Grammar::Python))
        );
    }

    #[test]
    fn typescript_and_tsx_grammars_parse_known_input() {
        assert!(CommentExtractor::new()
            .tree_spans(
                "const value: number = 1;\n",
                Grammar::TypeScript,
                TS.lexical
            )
            .is_ok());
        assert!(CommentExtractor::new()
            .tree_spans("const view = <div />;\n", Grammar::Tsx, TSX.lexical)
            .is_ok());
    }

    #[test]
    fn typescript_template_interpolation_comments_merge() {
        let text = "const value = `${\n// one\n// two\n// three\n// four\nvalue\n}`;\n";
        assert_eq!(blocks(text, TS), vec![(2, 5)]);
    }

    #[test]
    fn typescript_parser_matches_lexical_spans_for_valid_cases() {
        let cases = [
            "// one\n// two\nconst value = 1;\n",
            "const value = `// text`;\n",
            "const value = `${`${1}`}`;\n// one\n",
            "const label = \"Δ // text\";\n// one\n",
            "/// docs\nconst value = 1;\n",
            "const value = 1; // trailing\n",
        ];
        for text in cases {
            assert_eq!(spans(text, TS), lexical_spans(text, TS.lexical));
        }
    }

    #[test]
    fn typescript_parser_ignores_template_text_and_regular_expressions() {
        let text = "const template = `// text`;\nconst pattern = /\\/\\/text/;\n";
        assert!(spans(text, TS).is_empty());
    }

    #[test]
    fn tsx_comments_use_the_tsx_grammar() {
        let text = "const view = <div>{/* one */}</div>;\n// two\n";
        assert_eq!(spans(text, TSX), lexical_spans(text, TSX.lexical));
    }

    #[test]
    fn malformed_typescript_falls_back_to_lexical_extraction() {
        let cases = [
            "const value = `unterminated\n// one\n",
            "const value = \"unterminated\n// one\n",
            "/* unterminated\n// one\n",
            "const value = `${\n// one\n",
        ];
        for text in cases {
            let mut extractor = CommentExtractor::new();
            assert!(extractor
                .tree_spans(text, Grammar::TypeScript, TS.lexical)
                .is_err());
            assert_eq!(extractor.spans(text, TS), lexical_spans(text, TS.lexical));
        }
    }

    #[test]
    fn rust_tree_keeps_nested_doc_text_in_one_span() {
        let text = "//!//x\n//! b\nfn item() {}\n";
        assert_eq!(
            spans_to_json(text, RUST, &mut CommentExtractor::new()),
            r#"[{"start_line":1,"end_line":2,"kind":"doc","text":"//!//x\n//! b"}]"#
        );
    }

    #[test]
    fn unsupported_swift_uses_lexical_comments() {
        let spec = lang_for_extension("source.swift").unwrap();
        assert_eq!(spec.grammar, None);
        assert_eq!(
            blocks("//*******\n// one\n// two\n// three\nlet value = 1\n", spec),
            vec![(1, 4)]
        );
    }

    #[test]
    fn deeply_nested_typescript_does_not_overflow_the_stack() {
        let depth = 100_000;
        let text = format!(
            "const value = {}0{};\n",
            "[".repeat(depth),
            "]".repeat(depth)
        );
        assert!(spans(&text, TS).is_empty());
    }

    #[test]
    fn every_enabled_grammar_parses_comments_and_ignores_string_markers() {
        let slash = LexicalMode::Slash { is_rust: false };
        let hash = LexicalMode::Hash { is_python: false };
        let cases = [
            (
                Grammar::Rust,
                LexicalMode::Slash { is_rust: true },
                "// real\nlet value = \"// ignored\";\n",
            ),
            (
                Grammar::TypeScript,
                slash,
                "// real\nconst value = \"// ignored\";\n",
            ),
            (
                Grammar::Tsx,
                slash,
                "// real\nconst value = <div title=\"// ignored\" />;\n",
            ),
            (
                Grammar::JavaScript,
                slash,
                "// real\nconst value = \"// ignored\";\n",
            ),
            (
                Grammar::Go,
                slash,
                "// real\npackage main\nvar value = \"// ignored\"\n",
            ),
            (
                Grammar::C,
                slash,
                "// real\nconst char *value = \"// ignored\";\n",
            ),
            (
                Grammar::Cpp,
                slash,
                "// real\nconst char *value = \"// ignored\";\n",
            ),
            (
                Grammar::Java,
                slash,
                "// real\nclass Value { String value = \"// ignored\"; }\n",
            ),
            (
                Grammar::Kotlin,
                slash,
                "// real\nval value = \"// ignored\"\n",
            ),
            (
                Grammar::CSharp,
                slash,
                "// real\nclass Value { string value = \"// ignored\"; }\n",
            ),
            (
                Grammar::Zig,
                slash,
                "// real\nconst value = \"// ignored\";\n",
            ),
            (
                Grammar::Scala,
                slash,
                "// real\nobject Value { val value = \"// ignored\" }\n",
            ),
            (
                Grammar::ObjectiveC,
                slash,
                "// real\nconst char *value = \"// ignored\";\n",
            ),
            (
                Grammar::Python,
                LexicalMode::Hash { is_python: true },
                "# real\nvalue = \"# ignored\"\n",
            ),
            (Grammar::Bash, hash, "# real\nvalue='# ignored'\n"),
            (Grammar::Zsh, hash, "# real\nvalue='# ignored'\n"),
            (Grammar::Ruby, hash, "# real\nvalue = \"# ignored\"\n"),
            (Grammar::Toml, hash, "# real\nvalue = \"# ignored\"\n"),
            (Grammar::Yaml, hash, "# real\nvalue: \"# ignored\"\n"),
        ];
        for (grammar, lexical, text) in cases {
            let spans = CommentExtractor::new()
                .tree_spans(text, grammar, lexical)
                .unwrap();
            assert_eq!(spans.len(), 1, "{grammar:?}");
            assert_eq!(spans[0].start_line, 1, "{grammar:?}");
            assert_eq!(spans[0].end_line, 1, "{grammar:?}");
        }
    }

    #[test]
    fn routing_covers_every_supported_extension_and_shebang() {
        let cases = [
            ("rs", Some(Grammar::Rust)),
            ("ts", Some(Grammar::TypeScript)),
            ("tsx", Some(Grammar::Tsx)),
            ("js", Some(Grammar::JavaScript)),
            ("jsx", Some(Grammar::JavaScript)),
            ("mjs", Some(Grammar::JavaScript)),
            ("cjs", Some(Grammar::JavaScript)),
            ("go", Some(Grammar::Go)),
            ("swift", None),
            ("c", Some(Grammar::C)),
            ("h", Some(Grammar::Cpp)),
            ("cc", Some(Grammar::Cpp)),
            ("cpp", Some(Grammar::Cpp)),
            ("hpp", Some(Grammar::Cpp)),
            ("java", Some(Grammar::Java)),
            ("kt", Some(Grammar::Kotlin)),
            ("kts", Some(Grammar::Kotlin)),
            ("cs", Some(Grammar::CSharp)),
            ("zig", Some(Grammar::Zig)),
            ("scala", Some(Grammar::Scala)),
            ("m", Some(Grammar::ObjectiveC)),
            ("mm", None),
            ("py", Some(Grammar::Python)),
            ("sh", Some(Grammar::Bash)),
            ("bash", Some(Grammar::Bash)),
            ("zsh", Some(Grammar::Zsh)),
            ("rb", Some(Grammar::Ruby)),
            ("pl", None),
            ("toml", Some(Grammar::Toml)),
            ("yaml", Some(Grammar::Yaml)),
            ("yml", Some(Grammar::Yaml)),
        ];
        for (extension, grammar) in cases {
            assert_eq!(
                lang_for_extension(&format!("source.{extension}")).map(|spec| spec.grammar),
                Some(grammar)
            );
        }
        let shebangs = [
            ("#!/usr/bin/env python3\n", Some(Grammar::Python)),
            ("#!/bin/bash\n", Some(Grammar::Bash)),
            ("#!/usr/bin/bash5\n", Some(Grammar::Bash)),
            ("#!/usr/bin/env sh\n", Some(Grammar::Bash)),
            ("#!/bin/dash\n", Some(Grammar::Bash)),
            ("#!/bin/ash\n", Some(Grammar::Bash)),
            ("#!/bin/mksh\n", Some(Grammar::Bash)),
            ("#!/usr/bin/ksh93\n", Some(Grammar::Bash)),
            ("#!/bin/yash\n", Some(Grammar::Bash)),
            ("#!/bin/posh\n", Some(Grammar::Bash)),
            ("#!/bin/csh\n", None),
            ("#!/bin/tcsh\n", None),
            ("#!/usr/bin/tclsh8.6\n", None),
            ("#!/bin/zsh\n", Some(Grammar::Zsh)),
            ("#!/usr/bin/fish\n", None),
            ("#!/usr/bin/env fish python3\n", None),
            ("#!/usr/bin/env --chdir python fish\n", None),
            ("#!/usr/bin/env node\n", Some(Grammar::JavaScript)),
            ("#!/usr/bin/env -S zsh -f\n", Some(Grammar::Zsh)),
            ("#!/usr/bin/env -S -i python3\n", Some(Grammar::Python)),
            ("#!/usr/bin/env -S -iv python3\n", Some(Grammar::Python)),
            ("#!/usr/bin/env -iS python3\n", Some(Grammar::Python)),
            ("#!/usr/bin/env -S -ivuFOO python3\n", Some(Grammar::Python)),
            ("#!/usr/bin/env -S uv run python3\n", Some(Grammar::Python)),
            (
                "#!/usr/bin/env -S /usr/bin/uv run python3\n",
                Some(Grammar::Python),
            ),
            (
                "#!/usr/bin/env -S -- FOO=bar python3\n",
                Some(Grammar::Python),
            ),
            ("#!/usr/bin/env -S -i\\_python3\n", Some(Grammar::Python)),
            ("#!/usr/bin/env -S \\_python3\n", Some(Grammar::Python)),
            (
                "#!/usr/bin/env -S -P /usr/bin python3\n",
                Some(Grammar::Python),
            ),
            ("#!/usr/bin/env - python3\n", Some(Grammar::Python)),
            ("#!/usr/bin/env -S pyth'on'3 -O\n", Some(Grammar::Python)),
            ("#!/usr/bin/env -S \"python3\" -O\n", Some(Grammar::Python)),
            ("#!/usr/bin/env -Spython3 -O\n", Some(Grammar::Python)),
            (
                "#!/usr/bin/env --split-string=python3 -O\n",
                Some(Grammar::Python),
            ),
            (
                "#!/usr/bin/env -u PYTHON python3.12\n",
                Some(Grammar::Python),
            ),
            ("#!/usr/bin/env ruby\n", Some(Grammar::Ruby)),
            ("#!/home/shared/bin/ruby\n", Some(Grammar::Ruby)),
            ("#!/usr/bin/env perl\n", None),
            ("#!/home/shared/bin/perl\n", None),
        ];
        for (text, grammar) in shebangs {
            assert_eq!(
                lang_for_shebang(text).map(|spec| spec.grammar),
                Some(grammar)
            );
        }
        for text in [
            "#!/opt/tool --python\n",
            "#!/usr/bin/pwsh\n",
            "#!/usr/bin/env --mystery python fish\n",
            "#!/opt/bin/python3evil\n",
            "#!/opt/bin/ruby3beta\n",
            "#!/usr/bin/env -S pyth\\on3\n",
            "#!/usr/bin/env -S \"\" python3\n",
            "#!/usr/bin/env -S # python3\n",
            "#!/usr/bin/env --split-stringpython3\n",
            "#!/usr/bin/env FOO=bar -i python3\n",
            "#!/usr/bin/env -S =x python3\n",
            "#!/usr/bin/python3\u{00a0}-O\n",
            "#!/usr/bin/env -0 python3\n",
            "#!/usr/bin/env --null python3\n",
        ] {
            assert_eq!(lang_for_shebang(text), None);
        }
    }

    #[test]
    fn ruby_block_comments_are_extracted() {
        let spec = lang_for_extension("source.rb").unwrap();
        assert_eq!(
            blocks(
                "=begin\none\ntwo\nthree\nfour\nfive\n=end\nvalue = 1\n",
                spec
            ),
            vec![(1, 7)]
        );
    }

    #[test]
    fn grammar_fixtures_cover_focused_string_and_heredoc_cases() {
        let javascript = LanguageSpec {
            grammar: Some(Grammar::JavaScript),
            lexical: LexicalMode::Slash { is_rust: false },
        };
        let cpp = LanguageSpec {
            grammar: Some(Grammar::Cpp),
            lexical: LexicalMode::Slash { is_rust: false },
        };
        let objective_c = LanguageSpec {
            grammar: Some(Grammar::ObjectiveC),
            lexical: LexicalMode::Slash { is_rust: false },
        };
        let lexical_objective_cpp = LanguageSpec {
            grammar: None,
            lexical: LexicalMode::Slash { is_rust: false },
        };
        let bash = LanguageSpec {
            grammar: Some(Grammar::Bash),
            lexical: LexicalMode::Hash { is_python: false },
        };
        let zsh = LanguageSpec {
            grammar: Some(Grammar::Zsh),
            lexical: LexicalMode::Hash { is_python: false },
        };
        let python = LanguageSpec {
            grammar: Some(Grammar::Python),
            lexical: LexicalMode::Hash { is_python: true },
        };
        let toml = LanguageSpec {
            grammar: Some(Grammar::Toml),
            lexical: LexicalMode::Hash { is_python: false },
        };
        let yaml = LanguageSpec {
            grammar: Some(Grammar::Yaml),
            lexical: LexicalMode::Hash { is_python: false },
        };
        assert_eq!(
            spans(
                "const value = <div>{/* real */}</div>;\nconst ignored = \"// ignored\";\n",
                TSX
            )
            .len(),
            1
        );
        assert_eq!(
            spans(
                "const value = <div>{/* real */}</div>;\nconst ignored = \"// ignored\";\n",
                javascript
            )
            .len(),
            1
        );
        assert_eq!(
            spans("// real\nconst char *value = \"// ignored\";\n", cpp).len(),
            1
        );
        assert_eq!(
            spans(
                "// real\nconst char *value = \"// ignored\";\n",
                objective_c
            )
            .len(),
            1
        );
        assert_eq!(
            spans(
                "// real\nconst char *value = \"// ignored\";\n",
                lexical_objective_cpp
            )
            .len(),
            1
        );
        assert_eq!(
            spans("# real\ncat <<'EOF'\n# ignored\nEOF\n", bash).len(),
            1
        );
        assert_eq!(spans("# real\ncat <<'EOF'\n# ignored\nEOF\n", zsh).len(), 1);
        assert_eq!(spans("# real\nvalue = \"# ignored\"\n", python).len(), 1);
        assert_eq!(spans("# real\nvalue = \"# ignored\"\n", toml).len(), 1);
        assert_eq!(spans("# real\nvalue: \"# ignored\"\n", yaml).len(), 1);
    }
}
