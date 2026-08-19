use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};
use tool_sync::manifest::{self, ToolSource};

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Eq, PartialEq)]
enum Flow {
    Menu,
    Add,
    Update,
}

struct Installer {
    command: String,
    args: Vec<String>,
    preview_args: Vec<String>,
}

struct NewTool {
    name: String,
    platforms: Vec<String>,
    commands: Vec<String>,
    skills: Vec<String>,
    pi_package: Option<String>,
    url: String,
    revision: String,
    installer: Installer,
}

fn main() -> ExitCode {
    let flow = match parse_arguments(env::args().skip(1)) {
        Ok(flow) => flow,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut output = io::stdout();
    match run(flow, &mut input, &mut output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn parse_arguments(mut args: impl Iterator<Item = String>) -> Result<Flow> {
    let flow = match args.next().as_deref() {
        None => Flow::Menu,
        Some("add") => Flow::Add,
        Some("update") => Flow::Update,
        Some(other) => {
            return Err(format!(
                "unknown argument {other}; usage: tool-wizard [add|update]"
            ))
        }
    };
    if args.next().is_some() {
        return Err("too many arguments; usage: tool-wizard [add|update]".to_owned());
    }
    Ok(flow)
}

fn run(flow: Flow, input: &mut dyn BufRead, output: &mut dyn Write) -> Result<()> {
    let repository_root = repository_root()?;
    let manifest_path = repository_root.join("config/tools.toml");

    let flow = match flow {
        Flow::Menu => loop {
            let choice = ask(
                input,
                output,
                "1) add a tool  2) update pinned revisions",
                "",
            )?;
            match choice.as_str() {
                "1" | "add" => break Flow::Add,
                "2" | "update" => break Flow::Update,
                _ => print(output, "type 1 or 2")?,
            }
        },
        chosen => chosen,
    };

    let is_manifest_changed = match flow {
        Flow::Add => add_flow(&manifest_path, input, output)?,
        Flow::Update => update_flow(&manifest_path, input, output)?,
        Flow::Menu => unreachable!(),
    };

    if is_manifest_changed {
        offer_install(&repository_root, input, output);
    }
    Ok(())
}

fn repository_root() -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if !output.status.success() {
        return Err("run tool-wizard inside the agents repository".to_owned());
    }
    let root = String::from_utf8(output.stdout)
        .map_err(|error| format!("git printed a non-UTF-8 path: {error}"))?;
    Ok(PathBuf::from(root.trim()))
}

fn add_flow(manifest_path: &Path, input: &mut dyn BufRead, output: &mut dyn Write) -> Result<bool> {
    let url = loop {
        let value = ask(input, output, "Repository (URL or owner/repo)", "")?;
        if !value.is_empty() {
            break normalized_url(&value);
        }
    };
    let reference = ask(input, output, "Git ref", "HEAD")?;
    let revision = remote_revision(&url, &reference)?;
    print(output, &format!("pinned {revision}"))?;

    let name = ask(input, output, "Name", &default_name(&url))?;
    let platforms = ask_platforms(input, output)?;

    let checkout = inspect_repository(&url, &revision, &name, output);
    let detected_skills = checkout.as_deref().map(detected_skills).unwrap_or_default();
    let detected_installer = detected_installer(checkout.as_deref());
    if let Some(checkout) = &checkout {
        let _ = fs::remove_dir_all(checkout);
    }

    let picks = loop {
        let value = ask(
            input,
            output,
            "Provides: 1) commands  2) skills  3) Pi extension package (numbers like '1 3' or '1,3'; 'a' for all)",
            "",
        )?;
        match parse_selection(&value, 3) {
            Ok(picks) if !picks.is_empty() => break picks,
            Ok(_) => print(output, "select at least one")?,
            Err(error) => print(output, &error)?,
        }
    };

    let commands = if picks.contains(&0) {
        ask_paths(input, output, "Command paths in the repo (comma separated)")?
    } else {
        Vec::new()
    };
    let skills = if picks.contains(&1) {
        ask_skills(input, output, &detected_skills)?
    } else {
        Vec::new()
    };
    let pi_package = if picks.contains(&2) {
        Some(ask(input, output, "Pi package path in the repo", ".")?)
    } else {
        None
    };
    let installer = ask_installer(input, output, detected_installer)?;

    let tool = NewTool {
        name,
        platforms,
        commands,
        skills,
        pi_package,
        url,
        revision,
        installer,
    };

    let mut preview = DocumentMut::new();
    append_tool(&mut preview, &tool)?;
    print(output, &format!("\n{}", preview.to_string().trim_end()))?;
    if !is_yes(&ask(input, output, "Write to config/tools.toml?", "y")?) {
        return Ok(false);
    }

    let manifest_text = fs::read_to_string(manifest_path)
        .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
    let mut document = parse_document(&manifest_text, manifest_path)?;
    append_tool(&mut document, &tool)?;
    write_validated(manifest_path, &document.to_string())?;
    print(
        output,
        &format!("wrote {} to {}", tool.name, manifest_path.display()),
    )?;
    Ok(true)
}

fn update_flow(
    manifest_path: &Path,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<bool> {
    let manifest = manifest::load(manifest_path).map_err(|error| error.to_string())?;
    let entries: Vec<(String, String, String)> = manifest
        .tools
        .into_iter()
        .filter_map(|tool| match tool.source {
            ToolSource::Git { url, revision } => Some((tool.name, url, revision)),
            ToolSource::Embedded { .. } => None,
        })
        .collect();
    if entries.is_empty() {
        print(output, "the manifest has no Git entries")?;
        return Ok(false);
    }

    for (index, (name, url, revision)) in entries.iter().enumerate() {
        print(
            output,
            &format!("{}) {name}  {}  {url}", index + 1, short(revision)),
        )?;
    }
    let picks = loop {
        let value = ask(
            input,
            output,
            "Update which tools (numbers like '1 3' or '1,3'; 'a' for all)",
            "",
        )?;
        match parse_selection(&value, entries.len()) {
            Ok(picks) => break picks,
            Err(error) => print(output, &error)?,
        }
    };
    if picks.is_empty() {
        print(output, "nothing selected")?;
        return Ok(false);
    }

    let mut updates = Vec::new();
    for index in picks {
        let (name, url, revision) = &entries[index];
        let latest = remote_revision(url, "HEAD")?;
        if latest == *revision {
            print(output, &format!("{name} is up to date"))?;
        } else {
            print(
                output,
                &format!("{name}: {} -> {}", short(revision), short(&latest)),
            )?;
            updates.push((name.clone(), latest));
        }
    }
    if updates.is_empty() {
        return Ok(false);
    }
    if !is_yes(&ask(
        input,
        output,
        &format!("Write {} updated revision(s)?", updates.len()),
        "y",
    )?) {
        return Ok(false);
    }

    let manifest_text = fs::read_to_string(manifest_path)
        .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
    let mut document = parse_document(&manifest_text, manifest_path)?;
    for (name, revision) in &updates {
        if !set_revision(&mut document, name, revision) {
            return Err(format!(
                "cannot find a Git source for {name} in the manifest"
            ));
        }
    }
    write_validated(manifest_path, &document.to_string())?;
    print(output, &format!("wrote {}", manifest_path.display()))?;
    Ok(true)
}

fn offer_install(repository_root: &Path, input: &mut dyn BufRead, output: &mut dyn Write) {
    let is_dry_wanted = matches!(
        ask(input, output, "Run ./install.sh --dry-run?", "y"),
        Ok(value) if is_yes(&value)
    );
    if is_dry_wanted {
        if let Err(error) = run_install(repository_root, true) {
            let _ = print(output, &error);
            return;
        }
    }
    let is_apply_wanted = matches!(
        ask(input, output, "Apply ./install.sh?", "n"),
        Ok(value) if is_yes(&value)
    );
    if is_apply_wanted {
        if let Err(error) = run_install(repository_root, false) {
            let _ = print(output, &error);
        }
    }
}

fn run_install(repository_root: &Path, is_dry_run: bool) -> Result<()> {
    let mut command = Command::new(repository_root.join("install.sh"));
    command
        .env("REPO_TARGET", repository_root)
        .current_dir(repository_root);
    if is_dry_run {
        command.arg("--dry-run");
    }
    let status = command
        .status()
        .map_err(|error| format!("cannot run install.sh: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("install.sh failed with {status}"))
    }
}

fn ask(
    input: &mut dyn BufRead,
    output: &mut dyn Write,
    prompt: &str,
    default: &str,
) -> Result<String> {
    if default.is_empty() {
        write!(output, "{prompt}: ")
    } else {
        write!(output, "{prompt} [{default}]: ")
    }
    .map_err(|error| format!("cannot write a prompt: {error}"))?;
    output
        .flush()
        .map_err(|error| format!("cannot flush a prompt: {error}"))?;
    let mut line = String::new();
    let read = input
        .read_line(&mut line)
        .map_err(|error| format!("cannot read input: {error}"))?;
    if read == 0 {
        return Err("input ended".to_owned());
    }
    let value = line.trim();
    Ok(if value.is_empty() {
        default.to_owned()
    } else {
        value.to_owned()
    })
}

fn print(output: &mut dyn Write, line: &str) -> Result<()> {
    writeln!(output, "{line}").map_err(|error| format!("cannot write output: {error}"))
}

fn is_yes(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "y" | "yes")
}

fn short(revision: &str) -> &str {
    revision.get(..7).unwrap_or(revision)
}

fn normalized_url(input: &str) -> String {
    let trimmed = input.trim();
    let is_shorthand = !trimmed.contains("://")
        && !trimmed.starts_with("git@")
        && !trimmed.starts_with('/')
        && !trimmed.starts_with('.')
        && trimmed.split('/').filter(|part| !part.is_empty()).count() == 2
        && !trimmed.contains(' ');
    if is_shorthand {
        format!(
            "https://github.com/{}.git",
            trimmed.trim_end_matches(".git")
        )
    } else {
        trimmed.to_owned()
    }
}

fn default_name(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("tool")
        .trim_end_matches(".git")
        .to_owned()
}

fn remote_revision(url: &str, reference: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["ls-remote", "--", url, reference])
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git ls-remote failed for {url}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let revision = stdout
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().next())
        .unwrap_or("");
    let is_commit = matches!(revision.len(), 40 | 64)
        && revision
            .chars()
            .all(|character| character.is_ascii_hexdigit());
    if is_commit {
        Ok(revision.to_owned())
    } else {
        Err(format!("no commit for {reference} at {url}"))
    }
}

fn inspect_repository(
    url: &str,
    revision: &str,
    name: &str,
    output: &mut dyn Write,
) -> Option<PathBuf> {
    let destination = env::temp_dir().join(format!("tool-wizard-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&destination);
    match shallow_clone(url, revision, &destination) {
        Ok(()) => Some(destination),
        Err(error) => {
            let _ = print(
                output,
                &format!("note: cannot inspect the repository ({error})"),
            );
            let _ = fs::remove_dir_all(&destination);
            None
        }
    }
}

fn shallow_clone(url: &str, revision: &str, destination: &Path) -> Result<()> {
    let clone = Command::new("git")
        .args(["clone", "--quiet", "--depth", "1", "--", url])
        .arg(destination)
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if !clone.status.success() {
        return Err(String::from_utf8_lossy(&clone.stderr).trim().to_owned());
    }
    let head = Command::new("git")
        .arg("-C")
        .arg(destination)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if String::from_utf8_lossy(&head.stdout).trim() == revision {
        return Ok(());
    }
    let fetch = Command::new("git")
        .arg("-C")
        .arg(destination)
        .args(["fetch", "--quiet", "--depth", "1", "origin", revision])
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if !fetch.status.success() {
        return Err(String::from_utf8_lossy(&fetch.stderr).trim().to_owned());
    }
    let checkout = Command::new("git")
        .arg("-C")
        .arg(destination)
        .args(["checkout", "--quiet", "--detach", revision])
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if checkout.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&checkout.stderr).trim().to_owned())
    }
}

fn detected_skills(root: &Path) -> Vec<String> {
    let mut skills = Vec::new();
    collect_skill_directories(root, root, 0, &mut skills);
    skills.sort();
    skills
}

fn collect_skill_directories(
    root: &Path,
    directory: &Path,
    depth: usize,
    skills: &mut Vec<String>,
) {
    if depth > 4 {
        return;
    }
    if directory != root && directory.join("SKILL.md").is_file() {
        if let Ok(relative) = directory.strip_prefix(root) {
            skills.push(relative.to_string_lossy().into_owned());
        }
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_hidden = path
            .file_name()
            .and_then(|part| part.to_str())
            .is_some_and(|part| part.starts_with('.') || part == "node_modules");
        if path.is_dir() && !is_hidden {
            collect_skill_directories(root, &path, depth + 1, skills);
        }
    }
}

fn detected_installer(root: Option<&Path>) -> Installer {
    let Some(root) = root else {
        return no_op_installer();
    };
    if root.join("package-lock.json").is_file() {
        return Installer {
            command: "npm".to_owned(),
            args: vec!["ci".to_owned(), "--omit=dev".to_owned()],
            preview_args: vec![
                "ci".to_owned(),
                "--omit=dev".to_owned(),
                "--dry-run".to_owned(),
            ],
        };
    }
    if root.join("bun.lock").is_file() || root.join("bun.lockb").is_file() {
        return Installer {
            command: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), "bun install --frozen-lockfile".to_owned()],
            preview_args: vec!["-c".to_owned(), "bun --version".to_owned()],
        };
    }
    if root.join("install.sh").is_file() {
        return Installer {
            command: "./install.sh".to_owned(),
            args: Vec::new(),
            preview_args: vec!["--dry-run".to_owned()],
        };
    }
    no_op_installer()
}

fn no_op_installer() -> Installer {
    Installer {
        command: "/usr/bin/true".to_owned(),
        args: Vec::new(),
        preview_args: Vec::new(),
    }
}

fn ask_platforms(input: &mut dyn BufRead, output: &mut dyn Write) -> Result<Vec<String>> {
    loop {
        let value = ask(input, output, "Platforms: 1) macos  2) linux", "both")?;
        let platforms = match value.to_ascii_lowercase().as_str() {
            "both" | "b" | "1 2" | "1,2" | "12" => vec!["macos".to_owned(), "linux".to_owned()],
            "1" | "macos" => vec!["macos".to_owned()],
            "2" | "linux" => vec!["linux".to_owned()],
            _ => {
                print(output, "type 1, 2, or both")?;
                continue;
            }
        };
        return Ok(platforms);
    }
}

fn ask_paths(input: &mut dyn BufRead, output: &mut dyn Write, prompt: &str) -> Result<Vec<String>> {
    loop {
        let value = ask(input, output, prompt, "")?;
        let paths: Vec<String> = value
            .split(',')
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .collect();
        if paths.is_empty() {
            print(output, "give at least one path")?;
        } else {
            return Ok(paths);
        }
    }
}

fn ask_skills(
    input: &mut dyn BufRead,
    output: &mut dyn Write,
    detected: &[String],
) -> Result<Vec<String>> {
    if detected.is_empty() {
        return ask_paths(input, output, "Skill paths in the repo (comma separated)");
    }
    for (index, skill) in detected.iter().enumerate() {
        print(output, &format!("{}) {skill}", index + 1))?;
    }
    loop {
        let value = ask(
            input,
            output,
            "Skills (numbers like '1 3' or '1,3'; 'a' for all)",
            "",
        )?;
        match parse_selection(&value, detected.len()) {
            Ok(picks) if !picks.is_empty() => {
                return Ok(picks
                    .into_iter()
                    .map(|index| detected[index].clone())
                    .collect());
            }
            Ok(_) => print(output, "select at least one")?,
            Err(error) => print(output, &error)?,
        }
    }
}

fn ask_installer(
    input: &mut dyn BufRead,
    output: &mut dyn Write,
    detected: Installer,
) -> Result<Installer> {
    let shown = if detected.args.is_empty() {
        detected.command.clone()
    } else {
        format!("{} {}", detected.command, detected.args.join(" "))
    };
    let value = ask(input, output, "Installer", &shown)?;
    if value == shown {
        return Ok(detected);
    }
    let mut parts = value.split_whitespace().map(str::to_owned);
    let command = parts.next().unwrap_or_else(|| "/usr/bin/true".to_owned());
    let args: Vec<String> = parts.collect();
    let preview = ask(
        input,
        output,
        "Installer preview args (space separated)",
        "",
    )?;
    let preview_args: Vec<String> = preview.split_whitespace().map(str::to_owned).collect();
    Ok(Installer {
        command,
        args,
        preview_args,
    })
}

fn parse_selection(value: &str, count: usize) -> Result<Vec<usize>> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("a") || value.eq_ignore_ascii_case("all") {
        return Ok((0..count).collect());
    }
    let mut picks = Vec::new();
    for token in value
        .split(|character: char| character == ',' || character.is_whitespace())
        .filter(|token| !token.is_empty())
    {
        let index: usize = token
            .parse()
            .map_err(|_| format!("{token} is not a number"))?;
        if index == 0 || index > count {
            return Err(format!("{index} is out of range"));
        }
        if !picks.contains(&(index - 1)) {
            picks.push(index - 1);
        }
    }
    Ok(picks)
}

fn parse_document(text: &str, path: &Path) -> Result<DocumentMut> {
    text.parse::<DocumentMut>()
        .map_err(|error| format!("{} is not valid TOML: {error}", path.display()))
}

fn append_tool(document: &mut DocumentMut, tool: &NewTool) -> Result<()> {
    let mut table = Table::new();
    table.insert("name", toml_edit::value(tool.name.as_str()));
    table.insert("platforms", toml_edit::value(string_array(&tool.platforms)));
    table.insert("commands", toml_edit::value(string_array(&tool.commands)));
    if let Some(package) = &tool.pi_package {
        table.insert("pi_package", toml_edit::value(package.as_str()));
    }
    if !tool.skills.is_empty() {
        table.insert("skills", toml_edit::value(string_array(&tool.skills)));
    }

    let mut source = InlineTable::new();
    source.insert("url", tool.url.as_str().into());
    source.insert("revision", tool.revision.as_str().into());
    table.insert("source", toml_edit::value(source));

    let mut installer = InlineTable::new();
    installer.insert("command", tool.installer.command.as_str().into());
    installer.insert("args", Value::Array(string_array(&tool.installer.args)));
    installer.insert(
        "preview_args",
        Value::Array(string_array(&tool.installer.preview_args)),
    );
    table.insert("installer", toml_edit::value(installer));

    document
        .entry("tools")
        .or_insert(Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .ok_or_else(|| "the manifest key tools is not an array of tables".to_owned())?
        .push(table);
    Ok(())
}

fn string_array(values: &[String]) -> Array {
    let mut array = Array::new();
    for value in values {
        array.push(value.as_str());
    }
    array
}

fn set_revision(document: &mut DocumentMut, name: &str, revision: &str) -> bool {
    let Some(tables) = document
        .get_mut("tools")
        .and_then(Item::as_array_of_tables_mut)
    else {
        return false;
    };
    for table in tables.iter_mut() {
        if table.get("name").and_then(Item::as_str) != Some(name) {
            continue;
        }
        return match table.get_mut("source") {
            Some(Item::Value(Value::InlineTable(source))) => {
                if source.contains_key("revision") {
                    source.insert("revision", revision.into());
                    true
                } else if let Some(Value::InlineTable(git)) = source.get_mut("git") {
                    git.insert("revision", revision.into());
                    true
                } else {
                    false
                }
            }
            Some(Item::Table(source)) => {
                source.insert("revision", toml_edit::value(revision));
                true
            }
            _ => false,
        };
    }
    false
}

fn write_validated(manifest_path: &Path, candidate: &str) -> Result<()> {
    let staged = manifest_path.with_extension("toml.wizard");
    fs::write(&staged, candidate)
        .map_err(|error| format!("cannot write {}: {error}", staged.display()))?;
    let validation = manifest::load(&staged).map_err(|error| error.to_string());
    let _ = fs::remove_file(&staged);
    validation?;
    fs::write(manifest_path, candidate)
        .map_err(|error| format!("cannot write {}: {error}", manifest_path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicU64, Ordering};

    static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    fn fixture_dir(name: &str) -> PathBuf {
        let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!("tool-wizard-{name}-{id}-{}", std::process::id()));
        fs::create_dir_all(&root).expect("create fixture root");
        root
    }

    fn git(directory: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args([
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.test",
            ])
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn remote_with_skill(root: &Path) -> PathBuf {
        let remote = root.join("remote-repo");
        fs::create_dir_all(remote.join("skills/demo")).expect("create skill directory");
        fs::write(remote.join("skills/demo/SKILL.md"), "# demo\n").expect("write skill");
        git(&remote, &["init", "--quiet", "--initial-branch=main"]);
        git(&remote, &["add", "."]);
        git(&remote, &["commit", "--quiet", "-m", "first"]);
        remote
    }

    fn run_flow(
        flow: fn(&Path, &mut dyn BufRead, &mut dyn Write) -> Result<bool>,
        manifest: &Path,
        lines: &str,
    ) -> (Result<bool>, String) {
        let mut input = Cursor::new(lines.to_owned());
        let mut output = Vec::new();
        let result = flow(manifest, &mut input, &mut output);
        (result, String::from_utf8_lossy(&output).into_owned())
    }

    #[test]
    fn normalizes_shorthand_and_keeps_full_urls() {
        assert_eq!(
            normalized_url("OwaisQuadri/rag"),
            "https://github.com/OwaisQuadri/rag.git"
        );
        assert_eq!(
            normalized_url("OwaisQuadri/rag.git"),
            "https://github.com/OwaisQuadri/rag.git"
        );
        for kept in [
            "https://github.com/OwaisQuadri/rag.git",
            "git@github.com:OwaisQuadri/rag.git",
            "/tmp/local/repo",
            "./relative/repo",
        ] {
            assert_eq!(normalized_url(kept), kept);
        }
    }

    #[test]
    fn parses_selections_including_all_and_rejects_bad_tokens() {
        assert_eq!(parse_selection("a", 3).expect("all"), vec![0, 1, 2]);
        assert_eq!(parse_selection("3 1,1", 3).expect("picks"), vec![2, 0]);
        assert_eq!(parse_selection("", 3).expect("empty"), Vec::<usize>::new());
        assert!(parse_selection("0", 3).is_err());
        assert!(parse_selection("4", 3).is_err());
        assert!(parse_selection("x", 3).is_err());
    }

    #[test]
    fn add_flow_pins_the_remote_head_and_records_detected_skills() {
        let root = fixture_dir("add");
        let remote = remote_with_skill(&root);
        let head = git(&remote, &["rev-parse", "HEAD"]);
        let manifest = root.join("tools.toml");
        fs::write(&manifest, "").expect("seed manifest");

        let lines = format!("{}\n\n\n\n2\na\n\n\n", remote.display());
        let (result, output) = run_flow(add_flow, &manifest, &lines);
        assert!(result.expect("add flow succeeds"), "{output}");

        let loaded = manifest::load(&manifest).expect("manifest loads");
        assert_eq!(loaded.tools.len(), 1);
        let tool = &loaded.tools[0];
        assert_eq!(tool.name, "remote-repo");
        assert_eq!(tool.skills, [PathBuf::from("skills/demo")]);
        assert!(tool.commands.is_empty());
        assert!(matches!(
            &tool.source,
            ToolSource::Git { revision, .. } if *revision == head
        ));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn update_flow_moves_a_pinned_revision_to_the_remote_head() {
        let root = fixture_dir("update");
        let remote = remote_with_skill(&root);
        let old = git(&remote, &["rev-parse", "HEAD"]);
        let manifest = root.join("tools.toml");
        fs::write(
            &manifest,
            format!(
                r#"[[tools]]
name = "demo"
platforms = ["macos", "linux"]
commands = []
skills = ["skills/demo"]
source = {{ url = "{}", revision = "{old}" }}
installer = {{ command = "/usr/bin/true", args = [], preview_args = [] }}
"#,
                remote.display()
            ),
        )
        .expect("seed manifest");

        fs::write(remote.join("second"), "second\n").expect("write change");
        git(&remote, &["add", "."]);
        git(&remote, &["commit", "--quiet", "-m", "second"]);
        let new = git(&remote, &["rev-parse", "HEAD"]);
        assert_ne!(old, new);

        let (result, output) = run_flow(update_flow, &manifest, "a\ny\n");
        assert!(result.expect("update flow succeeds"), "{output}");

        let loaded = manifest::load(&manifest).expect("manifest loads");
        assert!(matches!(
            &loaded.tools[0].source,
            ToolSource::Git { revision, .. } if *revision == new
        ));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn update_flow_reports_an_up_to_date_entry_without_writing() {
        let root = fixture_dir("current");
        let remote = remote_with_skill(&root);
        let head = git(&remote, &["rev-parse", "HEAD"]);
        let manifest = root.join("tools.toml");
        let text = format!(
            r#"[[tools]]
name = "demo"
platforms = ["macos", "linux"]
commands = []
skills = ["skills/demo"]
source = {{ url = "{}", revision = "{head}" }}
installer = {{ command = "/usr/bin/true", args = [], preview_args = [] }}
"#,
            remote.display()
        );
        fs::write(&manifest, &text).expect("seed manifest");

        let (result, output) = run_flow(update_flow, &manifest, "a\n");
        assert!(!result.expect("update flow succeeds"), "{output}");
        assert!(output.contains("demo is up to date"), "{output}");
        assert_eq!(fs::read_to_string(&manifest).expect("manifest"), text);
        let _ = fs::remove_dir_all(&root);
    }
}
