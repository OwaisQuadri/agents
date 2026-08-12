use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::{value, DocumentMut};

use crate::drift::{Change, ChangeKind, Tool};
use crate::error::SyncError;
use crate::fsio;

pub struct AgentDef {
    pub name: String,
    pub description: String,
    pub developer_instructions: String,
    pub model: Option<String>,
}

/// Parses one Claude agent markdown file into a flat agent definition.
/// Takes the .md path; returns AgentDef with frontmatter name, description,
/// and model, and the body as developer_instructions.
///
/// # Errors
/// Io on read failure; ManifestInvalid on missing frontmatter or a missing
/// name or description key.
pub fn parse_agent_md(path: &Path) -> Result<AgentDef, SyncError> {
    let text = match fsio::read_opt(path)? {
        Some(text) => text,
        None => {
            return Err(SyncError::Io(
                path.to_path_buf(),
                std::io::ErrorKind::NotFound.into(),
            ))
        }
    };
    let mut name = None;
    let mut description = None;
    let mut model = None;
    let mut body_start = None;
    let mut is_in_frontmatter = false;
    let mut cursor = 0;
    for line in text.split_inclusive('\n') {
        cursor += line.len();
        let content = line.trim_end_matches('\n').trim_end_matches('\r');
        if content == "---" {
            if is_in_frontmatter {
                body_start = Some(cursor);
                break;
            }
            is_in_frontmatter = true;
        } else if is_in_frontmatter {
            if let Some((key, entry)) = content.split_once(':') {
                match key {
                    "name" => name = Some(entry.trim().to_string()),
                    "description" => description = Some(entry.trim().to_string()),
                    "model" => model = Some(entry.trim().to_string()),
                    _ => {}
                }
            }
        }
    }
    let Some(body_start) = body_start else {
        return Err(invalid(path, "has no closed frontmatter fence"));
    };
    let name = name
        .filter(|field| !field.is_empty())
        .ok_or_else(|| invalid(path, "frontmatter has no name"))?;
    let is_too_long_for_a_filename = name.len() > 255;
    let has_a_control_char = name.chars().any(char::is_control);
    if name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || is_too_long_for_a_filename
        || has_a_control_char
    {
        return Err(invalid(
            path,
            &format!("name {name} cannot name an output file"),
        ));
    }
    let description = description
        .filter(|field| !field.is_empty())
        .ok_or_else(|| invalid(path, "frontmatter has no description"))?;
    Ok(AgentDef {
        name,
        description,
        developer_instructions: text[body_start..].to_string(),
        model: model.filter(|field| !field.is_empty()),
    })
}

/// Renders an agent definition as a Codex agent TOML document.
/// Takes the definition; returns the TOML text.
///
/// # Errors
/// none
pub fn render_agent_toml(def: &AgentDef) -> String {
    let mut doc = DocumentMut::new();
    doc.insert("name", value(def.name.as_str()));
    doc.insert("description", value(def.description.as_str()));
    doc.insert(
        "developer_instructions",
        value(def.developer_instructions.as_str()),
    );
    if let Some(model) = def.model.as_deref().filter(|word| is_full_model_id(word)) {
        doc.insert("model", value(model));
    }
    doc.to_string()
}

/// Parses and validates every agents/<name>/<name>.md under a source dir.
/// Takes the source dir; returns the parsed AgentDefs in source order, or
/// the first ManifestInvalid found.
///
/// # Errors
/// Io on a directory read failure; ManifestInvalid on the first agent file
/// that fails name-safety or required-field validation.
pub fn parse_agents_dir(src_dir: &Path) -> Result<Vec<AgentDef>, SyncError> {
    agent_md_paths(src_dir)?.iter().map(|path| parse_agent_md(path)).collect()
}

/// Converges the Codex agents dir onto the repo agents dir.
/// Renders every agents/<name>/<name>.md into <dest>/<name>.toml and removes
/// dest files whose source agent no longer exists.
/// Takes the source dir, dest dir, and dry-run flag; returns the changes.
///
/// # Errors
/// Io, ManifestInvalid, BackupFailed, VerifyFailed.
pub fn sync_agents(src_dir: &Path, dest_dir: &Path, is_dry_run: bool) -> Result<Vec<Change>, SyncError> {
    let mut renders = Vec::new();
    for md_path in agent_md_paths(src_dir)? {
        let def = parse_agent_md(&md_path)?;
        renders.push((def.name.clone(), render_agent_toml(&def)));
    }
    if !is_dry_run {
        fs::create_dir_all(dest_dir).map_err(|err| SyncError::Io(dest_dir.to_path_buf(), err))?;
    }
    let mut changes = Vec::new();
    for (name, rendered) in &renders {
        let dest = dest_dir.join(format!("{name}.toml"));
        let existing = fsio::read_opt(&dest)?;
        if existing.as_deref() == Some(rendered.as_str()) {
            continue;
        }
        let kind = if existing.is_some() {
            ChangeKind::Update
        } else {
            ChangeKind::Add
        };
        fsio::write_verified(&dest, rendered, existing.as_deref(), is_dry_run)?;
        changes.push(Change {
            tool: Tool::Codex,
            server: name.clone(),
            kind,
        });
    }
    for orphan in dest_toml_paths(dest_dir)? {
        let Some(stem) = orphan.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if renders.iter().any(|(name, _)| name.as_str() == stem) {
            continue;
        }
        let Some(existing) = fsio::read_opt(&orphan)? else {
            continue;
        };
        if !is_managed_shape(&existing) {
            continue;
        }
        changes.push(Change {
            tool: Tool::Codex,
            server: stem.to_string(),
            kind: ChangeKind::Remove,
        });
        if is_dry_run {
            continue;
        }
        fsio::backup(&orphan)?;
        fs::remove_file(&orphan).map_err(|err| SyncError::Io(orphan.clone(), err))?;
    }
    Ok(changes)
}

fn invalid(path: &Path, offense: &str) -> SyncError {
    SyncError::ManifestInvalid(format!("agent file {} {}", path.display(), offense))
}

fn is_full_model_id(word: &str) -> bool {
    word.contains('-') || word.bytes().any(|byte| byte.is_ascii_digit())
}

fn agent_md_paths(src_dir: &Path) -> Result<Vec<PathBuf>, SyncError> {
    let entries = fs::read_dir(src_dir).map_err(|err| SyncError::Io(src_dir.to_path_buf(), err))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| SyncError::Io(src_dir.to_path_buf(), err))?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let md_path = entry.path().join(format!("{name}.md"));
        if md_path.is_file() {
            paths.push(md_path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn dest_toml_paths(dest_dir: &Path) -> Result<Vec<PathBuf>, SyncError> {
    let entries = match fs::read_dir(dest_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(SyncError::Io(dest_dir.to_path_buf(), err)),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| SyncError::Io(dest_dir.to_path_buf(), err))?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn is_managed_shape(text: &str) -> bool {
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return false;
    };
    let required = ["name", "description", "developer_instructions"];
    required.iter().all(|key| doc.get(key).is_some())
        && doc
            .as_table()
            .iter()
            .all(|(key, _)| key == "model" || required.contains(&key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mcp-sync-agents-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    const SPEC_TESTER_SHAPE: &str = "\
---
name: spec-tester
description: Use to execute natural-language test cases (mode confirm) or one attack-angle charter (mode break) against a runnable SUT(system under test) through its drive harness, fresh context.
tools: Read, Write, Bash, Grep, Glob
model: sonnet
---
You execute tests against a running system and report what actually happened. Executed
commands and their quoted output are the only truth; your prose never outranks them.

## input contract

The dispatch prompt carries \"mode\" and `drive_matrix`.
";

    fn write_agent(src_dir: &Path, name: &str, md: &str) {
        let dir = src_dir.join(name);
        fs::create_dir_all(dir.join("evals")).expect("create agent dir");
        fs::write(dir.join("evals").join("run.sh"), "#!/bin/sh\n").expect("seed evals file");
        fs::write(dir.join(format!("{name}.md")), md).expect("seed agent md");
    }

    fn agent_md(name: &str, model: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: {name} does one job.\ntools: Read, Bash\nmodel: {model}\n---\nYou are {name}.\n\nQuote \"output\" only.\n"
        )
    }

    fn parse_err(path: &Path) -> SyncError {
        match parse_agent_md(path) {
            Err(err) => err,
            Ok(_) => panic!("agent md unexpectedly parsed"),
        }
    }

    fn spec_def() -> AgentDef {
        AgentDef {
            name: "spec-tester".to_string(),
            description: "Use to execute \"confirm\" cases.".to_string(),
            developer_instructions:
                "You execute tests.\n\n## contract\n\nQuote \"output\" only.\n".to_string(),
            model: Some("sonnet".to_string()),
        }
    }

    #[test]
    fn parse_reads_spec_tester_shaped_frontmatter() {
        let dir = fixture("parse");
        let path = dir.join("spec-tester.md");
        fs::write(&path, SPEC_TESTER_SHAPE).expect("seed agent md");
        let def = parse_agent_md(&path).expect("fixture parses");
        assert_eq!(def.name, "spec-tester");
        assert!(
            def.description
                .starts_with("Use to execute natural-language test cases"),
            "{}",
            def.description
        );
        assert!(def.description.ends_with("fresh context."), "{}", def.description);
        assert_eq!(def.model.as_deref(), Some("sonnet"));
        let body = SPEC_TESTER_SHAPE.splitn(3, "---\n").nth(2).expect("fixture body");
        assert_eq!(def.developer_instructions, body);
    }

    #[test]
    fn parse_rejects_missing_fence_naming_the_file() {
        let dir = fixture("no-fence");
        for (file, md) in [
            ("plain.md", "no frontmatter at all\n"),
            ("unclosed.md", "---\nname: x\ndescription: y\n"),
        ] {
            let path = dir.join(file);
            fs::write(&path, md).expect("seed agent md");
            let err = parse_err(&path);
            assert!(matches!(err, SyncError::ManifestInvalid(_)));
            let text = err.to_string();
            assert!(text.contains(file), "{text}");
            assert!(text.contains("fence"), "{text}");
        }
    }

    #[test]
    fn parse_rejects_missing_name_or_description() {
        let dir = fixture("missing-keys");
        for (file, md, missing) in [
            ("no-name.md", "---\ndescription: d\n---\nbody\n", "name"),
            ("no-description.md", "---\nname: a\n---\nbody\n", "description"),
            ("empty-name.md", "---\nname:\ndescription: d\n---\nbody\n", "name"),
        ] {
            let path = dir.join(file);
            fs::write(&path, md).expect("seed agent md");
            let err = parse_err(&path);
            assert!(matches!(err, SyncError::ManifestInvalid(_)));
            let text = err.to_string();
            assert!(text.contains(file), "{text}");
            assert!(text.contains(&format!("frontmatter has no {missing}")), "{text}");
        }
    }

    #[test]
    fn parse_rejects_path_escaping_names() {
        let dir = fixture("bad-name");
        for bad in ["../evil", "a/b", "a\\b", "peer/.."] {
            let path = dir.join("agent.md");
            fs::write(&path, format!("---\nname: {bad}\ndescription: d\n---\nbody\n"))
                .expect("seed agent md");
            let err = parse_err(&path);
            assert!(matches!(err, SyncError::ManifestInvalid(_)));
            let text = err.to_string();
            assert!(text.contains("agent.md"), "{text}");
            assert!(text.contains("cannot name an output file"), "{text}");
        }
    }

    #[test]
    fn render_is_valid_toml_and_round_trips_fields() {
        let def = spec_def();
        let text = render_agent_toml(&def);
        let doc: DocumentMut = text.parse().expect("render parses as TOML");
        assert_eq!(doc["name"].as_str(), Some("spec-tester"));
        assert_eq!(doc["description"].as_str(), Some(def.description.as_str()));
        assert_eq!(
            doc["developer_instructions"].as_str(),
            Some(def.developer_instructions.as_str())
        );
        assert!(text.contains("\"\"\"") || text.contains("'''"), "{text}");
        let name_at = text.find("name = ").expect("name key");
        let description_at = text.find("description = ").expect("description key");
        let instructions_at = text
            .find("developer_instructions = ")
            .expect("instructions key");
        assert!(name_at < description_at, "{text}");
        assert!(description_at < instructions_at, "{text}");
    }

    #[test]
    fn render_omits_tier_models_and_keeps_full_ids() {
        for tier in ["opus", "sonnet"] {
            let def = AgentDef { model: Some(tier.to_string()), ..spec_def() };
            let doc: DocumentMut = render_agent_toml(&def).parse().expect("render parses");
            assert!(doc.get("model").is_none(), "{tier}");
        }
        let def = AgentDef { model: None, ..spec_def() };
        let doc: DocumentMut = render_agent_toml(&def).parse().expect("render parses");
        assert!(doc.get("model").is_none());
        let def = AgentDef { model: Some("gpt-5-codex".to_string()), ..spec_def() };
        let text = render_agent_toml(&def);
        let doc: DocumentMut = text.parse().expect("render parses");
        assert_eq!(doc["model"].as_str(), Some("gpt-5-codex"));
        let instructions_at = text
            .find("developer_instructions = ")
            .expect("instructions key");
        let model_at = text.rfind("model = ").expect("model key");
        assert!(instructions_at < model_at, "{text}");
    }

    #[test]
    fn sync_adds_updates_removes_and_skips() {
        let dir = fixture("sync");
        let src = dir.join("agents");
        let dest = dir.join("codex-agents");
        fs::create_dir_all(&dest).expect("create dest");
        write_agent(&src, "alpha", &agent_md("alpha", "sonnet"));
        write_agent(&src, "bravo", &agent_md("bravo", "opus"));
        write_agent(&src, "delta", &agent_md("delta", "sonnet"));
        fs::create_dir_all(src.join("scratch")).expect("create empty dir");
        fs::write(src.join("README.md"), "not an agent\n").expect("seed stray file");
        fs::write(
            dest.join("bravo.toml"),
            "name = \"bravo\"\ndescription = \"old\"\ndeveloper_instructions = \"old\"\n",
        )
        .expect("seed stale bravo");
        let delta = parse_agent_md(&src.join("delta").join("delta.md")).expect("delta parses");
        fs::write(dest.join("delta.toml"), render_agent_toml(&delta)).expect("seed current delta");
        fs::write(
            dest.join("charlie.toml"),
            "name = \"charlie\"\ndescription = \"gone\"\ndeveloper_instructions = \"gone\"\nmodel = \"gpt-5-codex\"\n",
        )
        .expect("seed orphan");
        fs::write(dest.join("notes.toml"), "kind = \"scratch\"\n").expect("seed foreign toml");
        fs::write(dest.join("readme.txt"), "not toml\n").expect("seed foreign file");

        let changes = sync_agents(&src, &dest, false).expect("sync succeeds");
        assert_eq!(
            crate::drift::render_plan(&changes, false),
            "plan: codex add alpha\n\
             plan: codex update bravo\n\
             plan: codex remove charlie\n"
        );

        let alpha: DocumentMut = fs::read_to_string(dest.join("alpha.toml"))
            .unwrap()
            .parse()
            .expect("alpha parses");
        assert_eq!(alpha["name"].as_str(), Some("alpha"));
        assert_eq!(alpha["description"].as_str(), Some("alpha does one job."));
        assert!(alpha.get("model").is_none());
        let bravo = fs::read_to_string(dest.join("bravo.toml")).unwrap();
        assert!(bravo.contains("bravo does one job."), "{bravo}");
        assert!(!dest.join("charlie.toml").exists());
        let names: Vec<String> = fs::read_dir(&dest)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.iter().any(|name| name.starts_with("charlie.toml.pre-sync-")),
            "{names:?}"
        );
        assert_eq!(
            fs::read_to_string(dest.join("notes.toml")).unwrap(),
            "kind = \"scratch\"\n"
        );
        assert_eq!(fs::read_to_string(dest.join("readme.txt")).unwrap(), "not toml\n");

        let again = sync_agents(&src, &dest, false).expect("second sync succeeds");
        assert!(again.is_empty(), "{}", crate::drift::render_plan(&again, false));
    }

    #[test]
    fn sync_dry_run_reports_and_touches_nothing() {
        let dir = fixture("dry");
        let src = dir.join("agents");
        let dest = dir.join("codex-agents");
        fs::create_dir_all(&dest).expect("create dest");
        write_agent(&src, "alpha", &agent_md("alpha", "sonnet"));
        fs::write(
            dest.join("charlie.toml"),
            "name = \"charlie\"\ndescription = \"gone\"\ndeveloper_instructions = \"gone\"\n",
        )
        .expect("seed orphan");

        let changes = sync_agents(&src, &dest, true).expect("dry sync succeeds");
        assert_eq!(
            crate::drift::render_plan(&changes, true),
            "dry:  codex add alpha\n\
             dry:  codex remove charlie\n"
        );
        assert!(!dest.join("alpha.toml").exists());
        assert!(dest.join("charlie.toml").exists());
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 1);

        let ghost = dir.join("ghost");
        let changes = sync_agents(&src, &ghost, true).expect("dry sync onto absent dir succeeds");
        assert_eq!(crate::drift::render_plan(&changes, true), "dry:  codex add alpha\n");
        assert!(!ghost.exists());
    }

    #[test]
    fn repo_agents_all_parse_and_render() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agents");
        let paths = agent_md_paths(&src).expect("repo agents dir lists");
        assert_eq!(paths.len(), 6, "{paths:?}");
        for path in paths {
            let def = parse_agent_md(&path).expect("repo agent parses");
            assert!(!def.description.is_empty(), "{}", path.display());
            assert_eq!(
                Some(def.name.as_str()),
                path.file_stem().and_then(|stem| stem.to_str())
            );
            let doc: DocumentMut = render_agent_toml(&def)
                .parse()
                .expect("repo agent renders valid TOML");
            assert_eq!(doc["name"].as_str(), Some(def.name.as_str()));
            assert_eq!(doc["description"].as_str(), Some(def.description.as_str()));
            assert_eq!(
                doc["developer_instructions"].as_str(),
                Some(def.developer_instructions.as_str())
            );
            assert!(doc.get("model").is_none(), "{}", path.display());
        }
    }
}
