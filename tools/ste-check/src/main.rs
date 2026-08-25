//! Scores one piece of prose against the Simplified Technical English rules in
//! docs/prompt-style.md, plus the extra rules of whichever register it is written for.
//!
//! The output shape is a contract, not a preference. The eval harnesses in
//! skills/*/evals/run.sh read `FAIL  <rule>: <offenders>` lines to cap a case at 4, and
//! read the trailing `score:` line for the GEPA feedback loop.

mod bro;
mod byline;
mod computah;
mod mask;
mod mouthpiece;
mod ste;
mod text;

use mask::Profile;
use ste::Rule;
use std::io::Read;
use std::process::ExitCode;

const USAGE: &str = "usage: ste-check --register <mouthpiece|computah|byline|bro|agent> [file]";

fn register_rules(name: &str) -> Option<(&'static [Rule], Profile)> {
    match name {
        "mouthpiece" => Some((mouthpiece::RULES, Profile::Spoken)),
        "computah" => Some((computah::RULES, Profile::Spoken)),
        "byline" => Some((byline::RULES, Profile::Shipped)),
        "bro" => Some((bro::RULES, Profile::Shipped)),
        "agent" => Some((&[], Profile::Shipped)),
        _ => None,
    }
}

fn read_input(path: Option<String>) -> std::io::Result<String> {
    match path {
        Some(path) => std::fs::read_to_string(path),
        None => {
            let mut buffer = String::new();
            std::io::stdin().read_to_string(&mut buffer)?;
            Ok(buffer)
        }
    }
}

fn report(name: &str, offenders: &[String]) {
    if offenders.is_empty() {
        println!("pass  {name}");
        return;
    }
    let shown: Vec<String> = offenders
        .iter()
        .take(3)
        .map(|o| format!("{:?}", o.replace(text::MASK, "\u{2026}").trim()))
        .collect();
    let more = if offenders.len() > 3 {
        format!(" (+{} more)", offenders.len() - 3)
    } else {
        String::new()
    };
    println!("FAIL  {name}: {}{more}", shown.join(", "));
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut register = None;
    let mut path = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--register" | "-r" => register = args.next(),
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => path = Some(arg),
        }
    }

    let Some(register) = register else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let Some((extra, profile)) = register_rules(&register) else {
        eprintln!("unknown register {register:?}\n{USAGE}");
        return ExitCode::from(2);
    };

    let text = match read_input(path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("cannot read input: {error}");
            return ExitCode::from(2);
        }
    };
    let masked = mask::exact(text::strip_frontmatter(&text), profile);

    let mut passed = 0;
    let total = ste::RULES.len() + extra.len();
    for (name, rule) in ste::RULES.iter().chain(extra.iter()) {
        let offenders = rule(&masked);
        if offenders.is_empty() {
            passed += 1;
        }
        report(name, &offenders);
    }
    println!("score: {passed}/{total} ({:.3})", passed as f64 / total as f64);

    if passed == total {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spoken(text: &str) -> String {
        mask::exact(text::strip_frontmatter(text), Profile::Spoken)
    }

    fn shipped(text: &str) -> String {
        mask::exact(text::strip_frontmatter(text), Profile::Shipped)
    }

    fn check(rule: fn(&str) -> Vec<String>, clean: &str, dirty: &str) {
        assert!(rule(&spoken(clean)).is_empty(), "clean case failed: {clean:?}");
        assert!(!rule(&spoken(dirty)).is_empty(), "dirty case passed: {dirty:?}");
    }

    #[test]
    fn sentence_length() {
        check(
            ste::sentence_length,
            "Run the check. It prints one line per rule.",
            "Run the check on every candidate message and then read the score line and the \
             failure lines and decide whether the register still holds together at all.",
        );
    }

    #[test]
    fn active_voice() {
        check(
            ste::active_voice,
            "The orchestrator appends the failure line.",
            "The failure lines are appended.",
        );
    }

    #[test]
    fn texting_shortforms() {
        check(ste::texting_shortforms, "Tell me if you want the diff.", "lmk if u want it.");
    }

    #[test]
    fn contraction_apostrophes() {
        check(
            ste::contraction_apostrophes,
            "I do not have the baseline.",
            "i dont have the baseline.",
        );
    }

    #[test]
    fn sentence_case() {
        check(
            ste::sentence_case,
            "The sweep is complete. No fact changed.",
            "the sweep is complete, no fact changed",
        );
    }

    /// The noun-stack rule shipped and came back out: it flagged "the checker masks four
    /// span kinds" and four more like it, against one true positive. Separating a noun
    /// stack from a noun-verb-noun clause needs part-of-speech data this tool has none of,
    /// so prompt-style.md keeps the rule for a human and the checker does not grade it.
    #[test]
    fn noun_stack_is_not_machine_checked() {
        let text = spoken("The checker masks four span kinds.");
        assert!(ste::RULES.iter().all(|(_, rule)| rule(&text).is_empty()));
    }

    #[test]
    fn dropped_relative_pronoun() {
        check(
            ste::dropped_relative_pronoun,
            "The angles that partition the failure space.",
            "The angles partitioning the failure space.",
        );
    }

    #[test]
    fn imperative_in_parenthetical() {
        check(
            ste::imperative_in_parenthetical,
            "Read the rubric (evals/rubric.md holds it).",
            "Read the rubric (you must run check.py first).",
        );
    }

    #[test]
    fn simple_word() {
        check(
            ste::simple_word,
            "Use the checker before the commit.",
            "Utilize the checker prior to the commit.",
        );
    }

    #[test]
    fn paragraph_length() {
        check(
            ste::paragraph_length,
            "One. Two. Three.",
            "One. Two. Three. Four. Five. Six. Seven.",
        );
    }

    #[test]
    fn mouthpiece_rules() {
        check(mouthpiece::not_just_but, "The cap is 500 characters.", "not just a cap, but a rule.");
        check(mouthpiece::timestamps, "The backfill ends at 5pm.", "The backfill ends at 14:32.");
        check(mouthpiece::plain_text, "The sweep is done.", "The **sweep** is done.");
        check(
            mouthpiece::list_cap,
            "1. one\n2. two\n3. three",
            "1. one\n2. two\n3. three\n4. four\n5. five\n6. six",
        );
    }

    #[test]
    fn mouthpiece_grades_the_bro_plain_words_pair() {
        let dirty = spoken("The PR is open and I dispatched the checker.");
        let failed = mouthpiece::RULES.iter().filter(|(_, rule)| !rule(&dirty).is_empty());
        assert_eq!(failed.count(), 2);
    }

    #[test]
    fn bare_acronym_counts_both_expansion_orders() {
        check(bro::jargon, "I sent the checker over every register.", "I dispatched the checker.");
        check(bro::bare_acronym, "The pull request is open.", "The PR is open.");
        check(bro::bare_acronym, "All checks pass on pull request (PR) 412.", "All checks pass on PR 412.");
        check(bro::bare_acronym, "The push landed before GATE E cleared.", "The push landed before ACK E.");
        check(
            bro::bare_acronym,
            "Simplified Technical English (STE) is a standard. Every reply runs on STE.",
            "Every reply runs on STE, and the sweep covers STE too.",
        );
        check(
            bro::bare_acronym,
            "The branch map/CPU-0003 landed, and ASD-STE100 is the standard.",
            "The CI run is red on the CI-Driven branch.",
        );
    }

    /// A parenthesis alone is not an expansion, so "green (CI)" must not license CI here or
    /// in any later sentence. Without the check the rule was weaker than it was before the
    /// mouthpiece register borrowed it.
    #[test]
    fn a_parenthesis_alone_does_not_expand_an_acronym() {
        check(
            bro::bare_acronym,
            "The Model Context Protocol (MCP) server died, and MCP is down.",
            "The run is green (CI). CI is red now.",
        );
    }

    #[test]
    fn exact_information_escapes_every_rule() {
        let text = "Run `python3 dont.py --utilize` and read /tmp/a/b now.";
        assert!(ste::texting_shortforms(&spoken(text)).is_empty());
        assert!(ste::contraction_apostrophes(&spoken(text)).is_empty());
        assert!(ste::simple_word(&spoken(text)).is_empty());
    }

    #[test]
    fn frontmatter_and_structure_are_not_prose() {
        let text = "---\nname: byline\ndescription: utilize the thing\n---\n\nThe pass is short.";
        assert!(ste::simple_word(&shipped(text)).is_empty());
        assert!(ste::sentence_case(&shipped("# heading\n\nThe pass is short.")).is_empty());
        assert!(ste::sentence_length(&shipped("| a | b |\n\nThe pass is short.")).is_empty());
    }

    #[test]
    fn bro_rules() {
        let clean = "Basically, the check runs twice. The first run finds the words that \
                     lose a reader, and the second run proves they are gone.";
        assert!(bro::RULES.iter().all(|(_, rule)| rule(&shipped(clean)).is_empty()));
        let dirty = shipped("It is worth noting that the DAG is idempotent.");
        assert!(bro::RULES.iter().any(|(_, rule)| !rule(&dirty).is_empty()));
    }

    /// An acronym inside a path or a backticked command is exact information, and the
    /// expansion rule must not reach it. Only the prose copy is graded.
    #[test]
    fn bro_expansion_rule_skips_exact_information() {
        let text = shipped("Read /tmp/DAG/notes and run `ls RAG` now.");
        assert!(bro::RULES.iter().all(|(_, rule)| rule(&text).is_empty()));
    }

    #[test]
    fn byline_rules() {
        let clean = "The installer links every skill. It backs up what it replaces first, \
                     and it never removes a path. Read install.sh for the order.";
        assert!(byline::RULES.iter().all(|(_, rule)| rule(&shipped(clean)).is_empty()));
        let dirty = shipped("It's worth noting that the pass is arguably quite robust.");
        assert!(byline::RULES.iter().any(|(_, rule)| !rule(&dirty).is_empty()));
    }
}
