use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

const REPOSITORY_ERROR: &str = "Blocked merge: could not resolve the target GitHub repository.";
const IDENTITY_ERROR: &str = "Blocked merge: could not identify the authenticated GitHub account.";
const INVALID_ARGUMENTS_ERROR: &str = "Blocked merge: invalid agent-merge-guard arguments.";
const GRAPHQL_INPUT_ERROR: &str =
    "Blocked merge: cannot inspect GraphQL input; use a readable file with a valid query.";

#[derive(Clone, Debug)]
enum RepositoryTarget {
    CurrentDirectory,
    Directory(PathBuf),
    ExplicitRepository(String),
    Invalid,
}

struct MergeAttempt {
    target: RepositoryTarget,
    is_graphql: bool,
    failure: Option<String>,
}

trait GitHubCli {
    fn run(&self, arguments: &[String], current_directory: &Path) -> Result<String, String>;
}

struct LiveGitHubCli;

impl GitHubCli for LiveGitHubCli {
    fn run(&self, arguments: &[String], current_directory: &Path) -> Result<String, String> {
        let output = Command::new("gh")
            .args(arguments)
            .current_dir(current_directory)
            .output()
            .map_err(|error| error.to_string())?;
        if output.status.success() {
            String::from_utf8(output.stdout).map_err(|error| error.to_string())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).into_owned())
        }
    }
}

fn shell_commands(command: &str) -> Vec<Vec<String>> {
    let (mut commands, mut token, mut characters) =
        (vec![Vec::new()], String::new(), command.chars().peekable());
    while let Some(character) = characters.next() {
        match character {
            ' ' | '\t' => push_token(&mut commands, &mut token),
            '\n' | '\r' => {
                push_token(&mut commands, &mut token);
                commands.push(Vec::new());
            }
            '\\' if characters.peek() == Some(&'\n') => {
                characters.next();
            }
            '\\' => {
                if let Some(escaped) = characters.next() {
                    token.push(escaped);
                }
            }
            '\'' | '"' => {
                for quoted_character in characters.by_ref() {
                    if quoted_character == character {
                        break;
                    }
                    token.push(quoted_character);
                }
            }
            ';' | '|' | '&' => {
                push_token(&mut commands, &mut token);
                commands.push(Vec::new());
                if characters.peek() == Some(&character) {
                    characters.next();
                }
            }
            '(' | '{' if token.is_empty() => commands.push(Vec::new()),
            ')' | '}' if token.is_empty() => commands.push(Vec::new()),
            _ => token.push(character),
        }
    }
    push_token(&mut commands, &mut token);
    commands
}

fn push_token(commands: &mut [Vec<String>], token: &mut String) {
    if !token.is_empty() {
        commands
            .last_mut()
            .expect("shell command collection is never empty")
            .push(std::mem::take(token));
    }
}

fn merge_attempts_in_directory(command: &str, current_directory: &Path) -> Vec<MergeAttempt> {
    let mut attempts = merge_attempts_with_target(command, current_directory, None);
    if is_directory_change_present(command) {
        for attempt in &mut attempts {
            attempt.target = RepositoryTarget::Invalid;
        }
    }
    attempts
}

fn is_directory_change_present(command: &str) -> bool {
    shell_commands(command)
        .iter()
        .any(|tokens| is_directory_change_command(tokens))
}

fn is_directory_change_command(tokens: &[String]) -> bool {
    let Some(index) = tokens.iter().position(|token| !token.contains('=')) else {
        return false;
    };
    let commands = &tokens[index..];
    match commands.first().map(|token| command_name(token)) {
        Some("cd") => true,
        Some("command" | "exec" | "then" | "do" | "else") => {
            is_directory_change_command(&commands[1..])
        }
        _ => false,
    }
}

fn merge_attempts_with_target(
    command: &str,
    current_directory: &Path,
    inherited_target: Option<RepositoryTarget>,
) -> Vec<MergeAttempt> {
    let mut directory = current_directory.to_path_buf();
    let mut attempts = Vec::new();
    for tokens in shell_commands(command) {
        if let Some(next_directory) = changed_directory(&tokens, &directory) {
            directory = next_directory;
        } else {
            attempts.extend(command_attempts(
                &tokens,
                &directory,
                inherited_target.clone(),
            ));
        }
    }
    attempts
}

fn changed_directory(tokens: &[String], current_directory: &Path) -> Option<PathBuf> {
    let command_index = tokens.iter().position(|token| !token.contains('='))?;
    if command_name(tokens.get(command_index)?) != "cd" {
        return None;
    }
    let path = tokens
        .get(command_index + 1 + usize::from(tokens.get(command_index + 1)?.as_str() == "--"))?;
    let path = Path::new(path);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_directory.join(path)
    })
}

fn merge_attempt(target: RepositoryTarget, is_graphql: bool) -> MergeAttempt {
    MergeAttempt {
        target,
        is_graphql,
        failure: None,
    }
}

fn command_attempts(
    tokens: &[String],
    current_directory: &Path,
    inherited_target: Option<RepositoryTarget>,
) -> Vec<MergeAttempt> {
    let Some(command_index) = tokens.iter().position(|token| !token.contains('=')) else {
        return Vec::new();
    };
    let target = repository_from_assignments(&tokens[..command_index]).or(inherited_target);
    let tokens = &tokens[command_index..];
    match tokens.first().map(|token| command_name(token)) {
        Some("bash" | "sh" | "zsh") => shell_wrapper_attempts(tokens, current_directory, target),
        Some("env") => env_wrapper_attempts(tokens, current_directory, target),
        Some("command" | "exec" | "nohup" | "sudo") => {
            command_wrapper_attempts(tokens, current_directory, target)
        }
        Some("if" | "elif" | "while" | "until" | "then" | "do" | "else" | "!") => {
            command_attempts(&tokens[1..], current_directory, target)
        }
        Some("git") => git_attempts(tokens, current_directory, target),
        Some("gh") => github_attempts(tokens, current_directory, target),
        _ => Vec::new(),
    }
}

fn repository_from_assignments(tokens: &[String]) -> Option<RepositoryTarget> {
    if let Some(repository) = tokens
        .iter()
        .rev()
        .find_map(|token| token.strip_prefix("GH_REPO="))
    {
        return Some(parse_repository(repository).map_or(
            RepositoryTarget::Invalid,
            RepositoryTarget::ExplicitRepository,
        ));
    }
    for name in ["GIT_DIR=", "GIT_WORK_TREE="] {
        if let Some(directory) = tokens
            .iter()
            .rev()
            .find_map(|token| token.strip_prefix(name))
        {
            return Some(RepositoryTarget::Directory(PathBuf::from(directory)));
        }
    }
    None
}

fn shell_wrapper_attempts(
    tokens: &[String],
    current_directory: &Path,
    target: Option<RepositoryTarget>,
) -> Vec<MergeAttempt> {
    let command_flag_index = tokens.iter().position(|token| {
        let flags = token.strip_prefix('-').unwrap_or_default();
        !flags.is_empty()
            && flags.contains('c')
            && flags.chars().all(|flag| matches!(flag, 'c' | 'i' | 'l'))
    });
    command_flag_index.map_or_else(Vec::new, |command_flag_index| {
        tokens.get(command_flag_index + 1).map_or_else(
            || vec![merge_attempt(RepositoryTarget::Invalid, false)],
            |nested_command| merge_attempts_with_target(nested_command, current_directory, target),
        )
    })
}

fn env_wrapper_attempts(
    tokens: &[String],
    current_directory: &Path,
    inherited_target: Option<RepositoryTarget>,
) -> Vec<MergeAttempt> {
    let mut index = 1;
    let mut command_directory = current_directory.to_path_buf();
    while let Some(token) = tokens.get(index) {
        if token == "--" {
            index += 1;
            break;
        }
        if !token.starts_with('-') {
            break;
        }
        if matches!(token.as_str(), "-C" | "--chdir") {
            let Some(directory) = tokens.get(index + 1) else {
                return vec![merge_attempt(RepositoryTarget::Invalid, false)];
            };
            command_directory = resolve_directory(&command_directory, directory);
            index += 2;
        } else if let Some(directory) = token.strip_prefix("--chdir=") {
            command_directory = resolve_directory(&command_directory, directory);
            index += 1;
        } else if matches!(token.as_str(), "-S" | "--split-string") {
            let Some(command) = tokens.get(index + 1) else {
                return vec![merge_attempt(RepositoryTarget::Invalid, false)];
            };
            return merge_attempts_with_target(command, &command_directory, inherited_target);
        } else if let Some(command) = token.strip_prefix("--split-string=") {
            return merge_attempts_with_target(command, &command_directory, inherited_target);
        } else {
            index += if matches!(token.as_str(), "-u" | "--unset") {
                2
            } else {
                1
            };
        }
    }
    let Some(command_index) = tokens[index..]
        .iter()
        .position(|token| !token.contains('='))
    else {
        return Vec::new();
    };
    let command_index = index + command_index;
    let target = repository_from_assignments(&tokens[index..command_index]).or(inherited_target);
    command_attempts(&tokens[command_index..], &command_directory, target)
}

fn command_wrapper_attempts(
    tokens: &[String],
    current_directory: &Path,
    inherited_target: Option<RepositoryTarget>,
) -> Vec<MergeAttempt> {
    let mut index = 1;
    while let Some(token) = tokens.get(index) {
        if token == "--" {
            index += 1;
            break;
        }
        if !token.starts_with('-') {
            break;
        }
        let wrapper = command_name(&tokens[0]);
        index += if (wrapper == "exec" && token == "-a")
            || (wrapper == "sudo"
                && matches!(
                    token.as_str(),
                    "-u" | "--user"
                        | "-g"
                        | "--group"
                        | "-h"
                        | "--host"
                        | "-p"
                        | "--prompt"
                        | "-C"
                        | "--close-from"
                        | "-R"
                        | "--chroot"
                        | "-D"
                        | "--chdir"
                )) {
            2
        } else {
            1
        };
    }
    command_attempts(&tokens[index..], current_directory, inherited_target)
}

fn resolve_directory(current_directory: &Path, directory: &str) -> PathBuf {
    let directory = Path::new(directory);
    if directory.is_absolute() {
        directory.to_path_buf()
    } else {
        current_directory.join(directory)
    }
}

fn command_name(token: &str) -> &str {
    Path::new(token.strip_prefix('\\').unwrap_or(token))
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(token)
}

fn git_attempts(
    tokens: &[String],
    current_directory: &Path,
    inherited_target: Option<RepositoryTarget>,
) -> Vec<MergeAttempt> {
    let mut index = 1;
    let mut command_directory = current_directory.to_path_buf();
    let mut target =
        inherited_target.unwrap_or_else(|| RepositoryTarget::Directory(command_directory.clone()));
    let mut is_git_dir_set = false;
    let mut command_aliases = Vec::new();
    while tokens
        .get(index)
        .is_some_and(|token| token.starts_with('-'))
    {
        let option = &tokens[index];
        if option == "-C" {
            let Some(directory) = tokens
                .get(index + 1)
                .filter(|directory| !directory.starts_with('-'))
            else {
                return vec![merge_attempt(RepositoryTarget::Invalid, false)];
            };
            command_directory = resolve_directory(&command_directory, directory);
            target = RepositoryTarget::Directory(command_directory.clone());
            index += 2;
        } else if let Some(directory) = option.strip_prefix("-C") {
            if directory.is_empty() {
                return vec![merge_attempt(RepositoryTarget::Invalid, false)];
            }
            command_directory = resolve_directory(&command_directory, directory);
            target = RepositoryTarget::Directory(command_directory.clone());
            index += 1;
        } else if let Some(directory) = option
            .strip_prefix("--work-tree=")
            .or_else(|| option.strip_prefix("--git-dir="))
        {
            if option.starts_with("--git-dir=") || !is_git_dir_set {
                target =
                    RepositoryTarget::Directory(resolve_directory(&command_directory, directory));
            }
            if option.starts_with("--git-dir=") {
                is_git_dir_set = true;
            }
            index += 1;
        } else if matches!(
            option.as_str(),
            "-c" | "--config"
                | "--config-env"
                | "--git-dir"
                | "--work-tree"
                | "--namespace"
                | "--super-prefix"
        ) {
            let Some(value) = tokens.get(index + 1) else {
                return vec![merge_attempt(RepositoryTarget::Invalid, false)];
            };
            if option == "-c" || option == "--config" {
                record_command_alias(value, &mut command_aliases);
            } else if option == "--git-dir" {
                target = RepositoryTarget::Directory(resolve_directory(&command_directory, value));
                is_git_dir_set = true;
            } else if option == "--work-tree" && !is_git_dir_set {
                target = RepositoryTarget::Directory(resolve_directory(&command_directory, value));
            }
            index += 2;
        } else if let Some(value) = option.strip_prefix("-c").filter(|value| !value.is_empty()) {
            record_command_alias(value, &mut command_aliases);
            index += 1;
        } else {
            index += 1;
        }
    }
    let Some(subcommand) = tokens.get(index).map(String::as_str) else {
        return Vec::new();
    };
    let arguments = &tokens[index + 1..];
    if command_aliases
        .iter()
        .any(|(name, value)| *name == subcommand && is_merge_alias(value, current_directory))
    {
        return vec![merge_attempt(target, false)];
    }
    let is_help = arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h");
    let is_merge = subcommand == "merge"
        && !is_help
        && !arguments
            .iter()
            .any(|argument| argument == "--abort" || argument == "--quit");
    let is_pull = subcommand == "pull"
        && !arguments.iter().any(|argument| {
            argument == "--ff-only"
                || argument == "-r"
                || argument == "--rebase"
                || matches!(
                    argument.strip_prefix("--rebase="),
                    Some(value) if !value.eq_ignore_ascii_case("false")
                )
        });
    if is_merge || is_pull {
        vec![merge_attempt(target, false)]
    } else {
        Vec::new()
    }
}

fn is_merge_alias(value: &str, current_directory: &Path) -> bool {
    let value = value.trim_start_matches('!');
    if !merge_attempts_with_target(value, current_directory, None).is_empty() {
        return true;
    }
    shell_commands(value).iter().any(|command| {
        command
            .first()
            .is_some_and(|token| matches!(command_name(token), "merge" | "pull"))
    })
}

fn record_command_alias<'a>(value: &'a str, aliases: &mut Vec<(&'a str, &'a str)>) {
    let Some((key, command)) = value.split_once('=') else {
        return;
    };
    let Some(name) = key.strip_prefix("alias.") else {
        return;
    };
    aliases.push((name, command));
}

fn parse_repository(repository: &str) -> Option<String> {
    let segments: Vec<_> = repository.split('/').collect();
    let (owner, name) = match segments.as_slice() {
        [owner, name] => (*owner, *name),
        ["github.com", owner, name] => (*owner, *name),
        _ => return None,
    };
    if owner.is_empty() || name.is_empty() {
        None
    } else {
        Some(format!("{owner}/{name}"))
    }
}

fn explicit_repository(tokens: &[String]) -> Option<RepositoryTarget> {
    for (index, token) in tokens.iter().enumerate() {
        if token == "--repo" || token == "-R" {
            return Some(
                tokens
                    .get(index + 1)
                    .and_then(|repository| parse_repository(repository))
                    .map_or(
                        RepositoryTarget::Invalid,
                        RepositoryTarget::ExplicitRepository,
                    ),
            );
        }
        if let Some(repository) = token.strip_prefix("--repo=").or_else(|| {
            token
                .strip_prefix("-R")
                .filter(|repository| !repository.is_empty())
        }) {
            return Some(parse_repository(repository).map_or(
                RepositoryTarget::Invalid,
                RepositoryTarget::ExplicitRepository,
            ));
        }
        if let Some(path) = token.strip_prefix("https://github.com/") {
            let mut segments = path.split('/');
            return Some(
                segments
                    .next()
                    .zip(segments.next())
                    .and_then(|(owner, name)| parse_repository(&format!("{owner}/{name}")))
                    .map_or(
                        RepositoryTarget::Invalid,
                        RepositoryTarget::ExplicitRepository,
                    ),
            );
        }
    }
    None
}

fn github_attempts(
    tokens: &[String],
    current_directory: &Path,
    inherited_target: Option<RepositoryTarget>,
) -> Vec<MergeAttempt> {
    let target = explicit_repository(tokens)
        .or(inherited_target)
        .unwrap_or(RepositoryTarget::CurrentDirectory);
    if tokens
        .windows(3)
        .any(|window| window == ["help", "pr", "merge"])
    {
        return Vec::new();
    }
    if is_pull_request_merge(tokens)
        && !tokens
            .iter()
            .any(|token| token == "--help" || token == "-h")
    {
        return vec![merge_attempt(target, false)];
    }
    if tokens.iter().enumerate().any(|(index, token)| {
        token == "api"
            && tokens[index + 1..]
                .iter()
                .any(|candidate| candidate == "graphql")
    }) {
        return graphql_attempt(tokens, target, current_directory)
            .into_iter()
            .collect();
    }
    if is_mutating_api_request(tokens) {
        for token in tokens {
            if let Some(repository) = merge_endpoint_repository(token) {
                let target = if repository == "{owner}/{repo}" {
                    target
                } else {
                    RepositoryTarget::ExplicitRepository(repository)
                };
                return vec![merge_attempt(target, false)];
            }
        }
    }
    Vec::new()
}

fn is_pull_request_merge(tokens: &[String]) -> bool {
    let Some(mut index) = tokens
        .iter()
        .position(|token| token == "pr")
        .map(|index| index + 1)
    else {
        return false;
    };
    while let Some(token) = tokens.get(index) {
        if matches!(token.as_str(), "-R" | "--repo") {
            index += 2;
        } else if token.starts_with('-') {
            index += 1;
        } else {
            return token == "merge";
        }
    }
    false
}

fn graphql_attempt(
    tokens: &[String],
    target: RepositoryTarget,
    current_directory: &Path,
) -> Option<MergeAttempt> {
    let queries = match graphql_queries(tokens, current_directory) {
        Ok(queries) => queries,
        Err(error) => {
            return Some(MergeAttempt {
                target: RepositoryTarget::Invalid,
                is_graphql: true,
                failure: Some(error),
            });
        }
    };
    let is_merge = tokens.iter().chain(queries.iter()).any(|value| {
        value.contains("mergePullRequest")
            || value.contains("enablePullRequestAutoMerge")
            || value.contains("enqueuePullRequest")
    });
    if !is_merge
        && tokens
            .iter()
            .any(|token| token.contains("query=$") || token.contains("query=`"))
    {
        return Some(MergeAttempt {
            target: RepositoryTarget::Invalid,
            is_graphql: true,
            failure: Some(GRAPHQL_INPUT_ERROR.to_owned()),
        });
    }
    is_merge.then(|| {
        let target = match target {
            RepositoryTarget::ExplicitRepository(repository) => {
                RepositoryTarget::ExplicitRepository(repository)
            }
            _ => RepositoryTarget::Invalid,
        };
        merge_attempt(target, true)
    })
}

fn graphql_queries(tokens: &[String], current_directory: &Path) -> Result<Vec<String>, String> {
    let mut queries = Vec::new();
    let mut index = 0;
    while let Some(token) = tokens.get(index) {
        let input = if token == "--input" || token == "-i" {
            index += 1;
            Some(
                tokens
                    .get(index)
                    .map(String::as_str)
                    .ok_or_else(|| GRAPHQL_INPUT_ERROR.to_owned())?,
            )
        } else {
            token
                .strip_prefix("--input=")
                .or_else(|| token.strip_prefix("-i").filter(|value| !value.is_empty()))
        };
        if let Some(input) = input {
            queries.push(graphql_input_query(input, current_directory)?);
        }
        if let Some(file) = query_file(token) {
            queries.push(graphql_file_query(file, current_directory)?);
        }
        index += 1;
    }
    Ok(queries)
}

fn query_file(token: &str) -> Option<&str> {
    token
        .strip_prefix("query=@")
        .or_else(|| token.strip_prefix("--field=query=@"))
        .or_else(|| token.strip_prefix("--raw-field=query=@"))
        .or_else(|| token.strip_prefix("-fquery=@"))
        .or_else(|| token.strip_prefix("-Fquery=@"))
}

fn graphql_input_query(input: &str, current_directory: &Path) -> Result<String, String> {
    let contents = graphql_file_contents(input, current_directory)?;
    let value: Value =
        serde_json::from_str(&contents).map_err(|_| GRAPHQL_INPUT_ERROR.to_owned())?;
    value
        .get("query")
        .and_then(Value::as_str)
        .filter(|query| !query.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| GRAPHQL_INPUT_ERROR.to_owned())
}

fn graphql_file_query(file: &str, current_directory: &Path) -> Result<String, String> {
    let query = graphql_file_contents(file, current_directory)?;
    (!query.trim().is_empty())
        .then_some(query)
        .ok_or_else(|| GRAPHQL_INPUT_ERROR.to_owned())
}

fn graphql_file_contents(file: &str, current_directory: &Path) -> Result<String, String> {
    if file == "-" {
        return Err(GRAPHQL_INPUT_ERROR.to_owned());
    }
    let path = Path::new(file);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_directory.join(path)
    };
    fs::read_to_string(path).map_err(|_| GRAPHQL_INPUT_ERROR.to_owned())
}

fn is_mutating_api_request(tokens: &[String]) -> bool {
    tokens.iter().enumerate().any(|(index, token)| {
        let method = if token == "--method" || token == "-X" {
            tokens.get(index + 1).map(String::as_str)
        } else if let Some(method) = token.strip_prefix("--method=") {
            Some(method)
        } else {
            token.strip_prefix("-X")
        };
        method.is_some_and(|method| {
            ["POST", "PUT", "PATCH", "DELETE"]
                .iter()
                .any(|candidate| method.eq_ignore_ascii_case(candidate))
        })
    })
}

fn merge_endpoint_repository(endpoint: &str) -> Option<String> {
    let endpoint = endpoint
        .strip_prefix("https://api.github.com/")
        .unwrap_or_else(|| endpoint.trim_start_matches('/'));
    let segments: Vec<_> = endpoint.split('?').next()?.split('/').collect();
    match segments.as_slice() {
        ["repos", owner, name, "pulls", _, "merge"] => parse_repository(&format!("{owner}/{name}")),
        _ => None,
    }
}

fn run_github_command(
    github: &dyn GitHubCli,
    arguments: &[&str],
    current_directory: &Path,
) -> Result<String, String> {
    github.run(
        &arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect::<Vec<_>>(),
        current_directory,
    )
}

fn repository_for_target(
    target: RepositoryTarget,
    current_directory: &Path,
    github: &dyn GitHubCli,
) -> Result<String, ()> {
    match target {
        RepositoryTarget::ExplicitRepository(repository) => parse_repository(&repository).ok_or(()),
        RepositoryTarget::CurrentDirectory => resolve_current_repository(current_directory, github),
        RepositoryTarget::Directory(directory) => {
            let directory = if directory.is_absolute() {
                directory
            } else {
                current_directory.join(directory)
            };
            resolve_current_repository(&directory, github)
        }
        RepositoryTarget::Invalid => Err(()),
    }
}

fn resolve_current_repository(directory: &Path, github: &dyn GitHubCli) -> Result<String, ()> {
    run_github_command(
        github,
        &[
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "--jq",
            ".nameWithOwner",
        ],
        directory,
    )
    .map_err(|_| ())
    .and_then(|repository| parse_repository(repository.trim()).ok_or(()))
}

fn check_attempt(
    attempt: MergeAttempt,
    current_directory: &Path,
    github: &dyn GitHubCli,
) -> Result<(), String> {
    if let Some(error) = attempt.failure {
        return Err(error);
    }
    let repository = repository_for_target(attempt.target, current_directory, github).map_err(|_| {
        if attempt.is_graphql {
            "Blocked merge: cannot resolve the repository for GraphQL merge; use gh pr merge --repo owner/name."
                .to_owned()
        } else {
            REPOSITORY_ERROR.to_owned()
        }
    })?;
    let owner = run_github_command(
        github,
        &["api", "user", "--jq", ".login"],
        current_directory,
    )
    .map_err(|_| IDENTITY_ERROR.to_owned())?;
    let owner = owner.trim();
    if owner.is_empty() || owner.contains(char::is_whitespace) {
        return Err(IDENTITY_ERROR.to_owned());
    }
    let contributors = run_github_command(
        github,
        &[
            "api",
            "--paginate",
            "--slurp",
            &format!("/repos/{repository}/contributors?anon=1&per_page=100"),
        ],
        current_directory,
    )
    .map_err(|_| format!("Blocked merge: could not fetch contributors for {repository}."))?;
    validate_contributors(&contributors, &repository, owner)?;
    let commits = run_github_command(
        github,
        &[
            "api",
            "--paginate",
            "--slurp",
            &format!("/repos/{repository}/commits?per_page=100"),
        ],
        current_directory,
    )
    .map_err(|_| format!("Blocked merge: could not verify commit authors for {repository}."))?;
    validate_commit_authors(&commits, &repository, owner)
}

fn validate_contributors(contributors: &str, repository: &str, owner: &str) -> Result<(), String> {
    let malformed_error =
        || format!("Blocked merge: GitHub returned malformed contributor data for {repository}.");
    let Value::Array(pages) = serde_json::from_str(contributors).map_err(|_| malformed_error())?
    else {
        return Err(malformed_error());
    };
    for page in pages {
        let Value::Array(contributors) = page else {
            return Err(malformed_error());
        };
        for contributor in contributors {
            let Some(contributor_type) = contributor.get("type").and_then(Value::as_str) else {
                return Err(malformed_error());
            };
            if contributor_type.eq_ignore_ascii_case("bot") {
                continue;
            }
            let login = match contributor.get("login") {
                None | Some(Value::Null) => {
                    return Err(format!(
                        "Blocked merge: anonymous contributor found in {repository}."
                    ));
                }
                Some(Value::String(login)) => login,
                Some(_) => return Err(malformed_error()),
            };
            if login.to_ascii_lowercase().ends_with("[bot]") {
                continue;
            }
            if !login.eq_ignore_ascii_case(owner) {
                return Err(format!(
                    "Blocked merge: {repository} has human contributor {login}."
                ));
            }
        }
    }
    Ok(())
}

fn validate_commit_authors(commits: &str, repository: &str, owner: &str) -> Result<(), String> {
    let malformed_error =
        || format!("Blocked merge: GitHub returned malformed commit data for {repository}.");
    let Value::Array(pages) = serde_json::from_str(commits).map_err(|_| malformed_error())? else {
        return Err(malformed_error());
    };
    for page in pages {
        let Value::Array(commits) = page else {
            return Err(malformed_error());
        };
        for commit in commits {
            let Some(author) = commit.get("author") else {
                return Err(malformed_error());
            };
            let Value::Object(author) = author else {
                if author.is_null() {
                    return Err(format!(
                        "Blocked merge: anonymous commit author found in {repository}."
                    ));
                }
                return Err(malformed_error());
            };
            let Some(author_type) = author.get("type").and_then(Value::as_str) else {
                return Err(malformed_error());
            };
            let Some(login) = author.get("login").and_then(Value::as_str) else {
                return Err(malformed_error());
            };
            if author_type.eq_ignore_ascii_case("bot")
                || login.to_ascii_lowercase().ends_with("[bot]")
                || login.eq_ignore_ascii_case(owner)
            {
                continue;
            }
            return Err(format!(
                "Blocked merge: {repository} has human commit author {login}."
            ));
        }
    }
    Ok(())
}

fn parse_cli(arguments: &[String]) -> Result<(String, PathBuf), &'static str> {
    if arguments.len() != 4
        || arguments.first().map(String::as_str) != Some("--check")
        || arguments.get(2).map(String::as_str) != Some("--cwd")
        || arguments.get(1).is_none_or(String::is_empty)
        || arguments
            .get(3)
            .is_none_or(|directory| directory.is_empty() || directory.starts_with('-'))
    {
        return Err(INVALID_ARGUMENTS_ERROR);
    }
    Ok((arguments[1].clone(), PathBuf::from(&arguments[3])))
}

fn main() -> ExitCode {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let (command, current_directory) = match parse_cli(&arguments) {
        Ok(arguments) => arguments,
        Err(error) => {
            println!("{error}");
            return ExitCode::FAILURE;
        }
    };
    for attempt in merge_attempts_in_directory(&command, &current_directory) {
        if let Err(error) = check_attempt(attempt, &current_directory, &LiveGitHubCli) {
            println!("{error}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque};

    struct FakeGitHubCli {
        responses: RefCell<VecDeque<Result<String, String>>>,
        calls: RefCell<Vec<(Vec<String>, PathBuf)>>,
    }

    impl FakeGitHubCli {
        fn new(responses: impl IntoIterator<Item = Result<&'static str, &'static str>>) -> Self {
            Self {
                responses: RefCell::new(
                    responses
                        .into_iter()
                        .map(|response| response.map(str::to_owned).map_err(str::to_owned))
                        .collect(),
                ),
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl GitHubCli for FakeGitHubCli {
        fn run(&self, arguments: &[String], current_directory: &Path) -> Result<String, String> {
            self.calls
                .borrow_mut()
                .push((arguments.to_vec(), current_directory.to_path_buf()));
            self.responses
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Err("unexpected gh command".to_owned()))
        }
    }

    fn owner_page() -> &'static str {
        r#"[[{"type":"User","login":"owner"}]]"#
    }

    fn owner_commits() -> &'static str {
        r#"[[{"author":{"type":"User","login":"owner"}}]]"#
    }

    fn allowed_responses() -> [Result<&'static str, &'static str>; 4] {
        [
            Ok("owner/name\n"),
            Ok("owner\n"),
            Ok(owner_page()),
            Ok(owner_commits()),
        ]
    }

    fn allowed_explicit_repository_responses() -> [Result<&'static str, &'static str>; 3] {
        [Ok("owner\n"), Ok(owner_page()), Ok(owner_commits())]
    }

    fn guard(
        command: &str,
        current_directory: &Path,
        github: &dyn GitHubCli,
    ) -> Result<(), String> {
        for attempt in merge_attempts_in_directory(command, current_directory) {
            check_attempt(attempt, current_directory, github)?;
        }
        Ok(())
    }

    #[test]
    fn allows_harmless_commands_without_calling_github() {
        let github = FakeGitHubCli::new([]);
        assert_eq!(
            guard("git status && gh pr view", Path::new("/workspace"), &github),
            Ok(())
        );
        assert!(github.calls.borrow().is_empty());
    }

    #[test]
    fn detects_git_merge_and_pull_commands() {
        for command in [
            "git merge topic",
            "git pull origin main",
            "git pull --rebase=false origin main",
            "git pull --no-rebase origin main",
        ] {
            let github = FakeGitHubCli::new(allowed_responses());
            assert_eq!(guard(command, Path::new("/workspace"), &github), Ok(()));
        }
    }

    #[test]
    fn allows_git_merge_base_abort_quit_and_safe_pull_modes() {
        for command in [
            "git merge-base HEAD origin/main",
            "git merge --abort",
            "git merge --quit",
            "git pull --ff-only",
            "git pull --rebase",
            "git pull --rebase=true",
            "git pull --rebase=merges",
            "git pull -r",
        ] {
            let github = FakeGitHubCli::new([]);
            assert_eq!(
                guard(command, Path::new("/workspace"), &github),
                Ok(()),
                "{command}"
            );
            assert!(github.calls.borrow().is_empty(), "{command}");
        }
    }

    #[test]
    fn resolves_relative_git_c_directory_against_tool_call_directory() {
        for command in [
            "git -C nested/repository merge topic",
            "git -Cnested/repository merge topic",
        ] {
            let github = FakeGitHubCli::new(allowed_responses());
            assert_eq!(
                guard(command, Path::new("/workspace"), &github),
                Ok(()),
                "{command}"
            );
            assert_eq!(
                github.calls.borrow()[0].1,
                PathBuf::from("/workspace/nested/repository"),
                "{command}"
            );
        }
    }

    #[test]
    fn directory_chains_fail_closed_and_explicit_directory_options_resolve() {
        for command in [
            "cd other/project && git merge topic",
            "(cd child && git status); git merge topic",
            "if true; then cd child; fi; git merge topic",
            "command cd child && git merge topic",
        ] {
            let github = FakeGitHubCli::new([]);
            assert_eq!(
                guard(command, Path::new("/workspace"), &github),
                Err(REPOSITORY_ERROR.to_owned()),
                "{command}"
            );
            assert!(github.calls.borrow().is_empty(), "{command}");
        }

        for command in [
            "env -C /other/project git merge topic",
            "env --chdir=/other/project git merge topic",
            "git --work-tree /other/project merge topic",
            "git --git-dir=/other/project/.git merge topic",
        ] {
            let github = FakeGitHubCli::new(allowed_responses());
            assert_eq!(
                guard(command, Path::new("/workspace"), &github),
                Ok(()),
                "{command}"
            );
            assert!(
                github.calls.borrow()[0].1.starts_with("/other/project"),
                "{command}"
            );
        }

        let github = FakeGitHubCli::new(allowed_responses());
        assert_eq!(
            guard(
                "git --git-dir=/repo-a/.git --work-tree=/repo-b merge topic",
                Path::new("/workspace"),
                &github
            ),
            Ok(())
        );
        assert_eq!(github.calls.borrow()[0].1, PathBuf::from("/repo-a/.git"));

        let github = FakeGitHubCli::new(allowed_responses());
        assert_eq!(
            guard(
                "echo cd && git merge topic",
                Path::new("/workspace"),
                &github
            ),
            Ok(())
        );
    }

    #[test]
    fn detects_wrapped_and_nested_merge_commands() {
        for command in [
            "/bin/zsh -lc 'zsh -lc \"git merge topic\"'",
            "/usr/local/bin/zsh -lc 'git merge topic'",
            "bash -lc 'git merge topic'",
            "sh -c 'git merge topic'",
            "env git merge topic",
            "env -u UNUSED git merge topic",
            "env -S 'git merge topic'",
            "env --split-string='git merge topic'",
            "nohup git merge topic",
            "sudo git merge topic",
            "sudo -u root git merge topic",
            "command git merge topic",
            "command -- git merge topic",
            "exec git merge topic",
            "exec -- git merge topic",
            "echo ready\ngit merge topic",
            "if true; then git merge topic; fi",
            "while false; do git merge topic; done",
            r#"git \
merge topic"#,
            "(git merge topic)",
            "{ git merge topic; }",
            "/usr/bin/git merge topic",
            "\\git merge topic",
            "git -c advice.detachedHead=false merge topic",
            "git -cadvice.detachedHead=false merge topic",
            "git --git-dir .git merge topic",
            "git --git-dir=.git merge topic",
            "git --work-tree /workspace merge topic",
            "git --work-tree=/workspace merge topic",
            "git -c alias.integrate='merge --no-edit' integrate topic",
            "git -c alias.integrate='!echo prep; git merge topic' integrate",
            "git -c alias.sync='pull origin main' sync",
        ] {
            let github = FakeGitHubCli::new(allowed_responses());
            assert_eq!(
                guard(command, Path::new("/workspace"), &github),
                Ok(()),
                "{command}"
            );
            assert!(!github.calls.borrow().is_empty(), "{command}");
        }
    }

    #[test]
    fn command_scoped_repository_selects_the_checked_repository() {
        for command in [
            "GH_REPO=other/project gh pr merge 7",
            "env GH_REPO=other/project gh pr merge 7",
        ] {
            let github = FakeGitHubCli::new(allowed_explicit_repository_responses());
            assert_eq!(
                guard(command, Path::new("/workspace"), &github),
                Ok(()),
                "{command}"
            );
            assert_eq!(
                github.calls.borrow()[1].0.last().map(String::as_str),
                Some("/repos/other/project/contributors?anon=1&per_page=100"),
                "{command}"
            );
        }

        for command in [
            "GIT_DIR=/other/project/.git git merge topic",
            "GIT_WORK_TREE=/other/project git merge topic",
        ] {
            let github = FakeGitHubCli::new(allowed_responses());
            assert_eq!(
                guard(command, Path::new("/workspace"), &github),
                Ok(()),
                "{command}"
            );
            assert!(
                github.calls.borrow()[0].1.starts_with("/other/project"),
                "{command}"
            );
        }
    }

    #[test]
    fn merge_help_commands_do_not_call_github() {
        for command in [
            "git merge --help",
            "git merge -h",
            "gh pr merge --help",
            "gh help pr merge",
        ] {
            let github = FakeGitHubCli::new([]);
            assert_eq!(
                guard(command, Path::new("/workspace"), &github),
                Ok(()),
                "{command}"
            );
            assert!(github.calls.borrow().is_empty(), "{command}");
        }
    }

    #[test]
    fn detects_gh_pr_merge_auto_and_repository_selectors() {
        for command in [
            "gh pr merge 7",
            "gh pr merge 7 --auto",
            "gh pr merge 7 --repo owner/name",
            "gh pr -R owner/name merge 7",
            "gh pr -Rowner/name merge 7",
            "gh pr -Rgithub.com/owner/name merge 7",
            "gh pr merge 7 -R owner/name",
            "gh pr merge 7 --repo=owner/name",
            "gh pr merge https://github.com/owner/name/pull/7",
        ] {
            let responses = if command.contains("--repo")
                || command.contains(" -R")
                || command.contains("github.com")
            {
                allowed_explicit_repository_responses().to_vec()
            } else {
                allowed_responses().to_vec()
            };
            let github = FakeGitHubCli::new(responses);
            assert_eq!(
                guard(command, Path::new("/workspace"), &github),
                Ok(()),
                "{command}"
            );
        }
    }

    #[test]
    fn blocks_malformed_gh_repository_selectors() {
        for command in [
            "gh pr merge --repo",
            "gh pr merge --repo=owner",
            "gh pr merge -R owner",
        ] {
            let github = FakeGitHubCli::new([]);
            assert_eq!(
                guard(command, Path::new("/workspace"), &github),
                Err(REPOSITORY_ERROR.to_owned()),
                "{command}"
            );
        }
    }

    #[test]
    fn classifies_only_exact_mutating_rest_api_methods() {
        for command in [
            "gh api --method PUT /repos/owner/name/pulls/7/merge",
            "gh api --method PUT '/repos/owner/name/pulls/7/merge?merge_method=squash'",
            "gh api --method PUT https://api.github.com/repos/owner/name/pulls/7/merge",
            "GH_REPO=owner/name gh api --method PUT /repos/{owner}/{repo}/pulls/7/merge",
            "gh api --method=PUT /repos/owner/name/pulls/7/merge",
            "gh api -X PUT /repos/owner/name/pulls/7/merge",
            "gh api -XPUT /repos/owner/name/pulls/7/merge",
            "/usr/local/bin/gh api --method=put /repos/owner/name/pulls/7/merge",
        ] {
            let github = FakeGitHubCli::new(allowed_explicit_repository_responses());
            assert_eq!(
                guard(command, Path::new("/workspace"), &github),
                Ok(()),
                "{command}"
            );
        }
        for command in [
            "gh api --method=GET /repos/owner/name/pulls/7/merge",
            "gh api -X GET /repos/owner/name/pulls/7/merge",
            "gh api /repos/owner/name/pulls/7/merge",
        ] {
            let github = FakeGitHubCli::new([]);
            assert_eq!(
                guard(command, Path::new("/workspace"), &github),
                Ok(()),
                "{command}"
            );
            assert!(github.calls.borrow().is_empty(), "{command}");
        }
    }

    #[test]
    fn graphql_merges_require_explicit_repository() {
        let github = FakeGitHubCli::new([]);
        assert_eq!(
            guard(
                "gh api -f query=mutation{mergePullRequest(input:$input){clientMutationId}} graphql",
                Path::new("/workspace"),
                &github,
            ),
            Err("Blocked merge: cannot resolve the repository for GraphQL merge; use gh pr merge --repo owner/name.".to_owned())
        );
        assert!(github.calls.borrow().is_empty());

        let github = FakeGitHubCli::new([]);
        assert_eq!(
            guard(
                "gh api graphql --repo owner/name -f query=$QUERY",
                Path::new("/workspace"),
                &github,
            ),
            Err(GRAPHQL_INPUT_ERROR.to_owned())
        );
        assert!(github.calls.borrow().is_empty());

        let github = FakeGitHubCli::new(allowed_explicit_repository_responses());
        assert_eq!(
            guard(
                "gh api graphql --repo owner/name -f query=mutation{enqueuePullRequest(input:$input){clientMutationId}}",
                Path::new("/workspace"),
                &github,
            ),
            Ok(())
        );
    }

    #[test]
    fn graphql_file_inputs_are_inspected() {
        let directory =
            std::env::temp_dir().join(format!("agent-merge-guard-graphql-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("harmless.json"),
            r#"{"query":"query { viewer { login } }"}"#,
        )
        .unwrap();
        fs::write(
            directory.join("merge.json"),
            r#"{"query":"mutation { mergePullRequest(input: {}) { clientMutationId } }"}"#,
        )
        .unwrap();
        fs::write(
            directory.join("merge.graphql"),
            "mutation { enablePullRequestAutoMerge(input: {}) { clientMutationId } }",
        )
        .unwrap();

        let github = FakeGitHubCli::new([]);
        assert_eq!(
            guard("gh api graphql --input harmless.json", &directory, &github),
            Ok(())
        );
        assert!(github.calls.borrow().is_empty());

        for command in [
            "gh api graphql --repo owner/name --input merge.json",
            "gh api graphql --repo owner/name -F query=@merge.graphql",
            "gh api graphql --repo owner/name -Fquery=@merge.graphql",
        ] {
            let github = FakeGitHubCli::new(allowed_explicit_repository_responses());
            assert_eq!(guard(command, &directory, &github), Ok(()), "{command}");
        }

        for command in [
            "gh api graphql --input -",
            "gh api graphql --input missing.json",
            "gh api graphql --input harmless.graphql",
        ] {
            let github = FakeGitHubCli::new([]);
            assert_eq!(
                guard(command, &directory, &github),
                Err(GRAPHQL_INPUT_ERROR.to_owned()),
                "{command}"
            );
            assert!(github.calls.borrow().is_empty(), "{command}");
        }

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn allows_owner_and_bot_contributors() {
        for contributors in [
            r#"[[{"type":"User","login":"owner"}]]"#,
            r#"[[{"type":"User","login":"owner"},{"type":"Bot","login":"automation"}]]"#,
            r#"[[{"type":"User","login":"owner"},{"type":"User","login":"automation[bot]"}]]"#,
            r#"[[{"type":"User","login":"OWNER"}]]"#,
            "[]",
        ] {
            let github = FakeGitHubCli::new([
                Ok("owner/name\n"),
                Ok("owner\n"),
                Ok(contributors),
                Ok(owner_commits()),
            ]);
            assert_eq!(
                guard("git merge topic", Path::new("/workspace"), &github),
                Ok(()),
                "{contributors}"
            );
        }
    }

    #[test]
    fn fresh_commit_authors_close_the_contributor_cache_window() {
        for (commits, expected) in [
            (
                r#"[[{"author":{"type":"User","login":"other"}}]]"#,
                "Blocked merge: owner/name has human commit author other.",
            ),
            (
                r#"[[{"author":null}]]"#,
                "Blocked merge: anonymous commit author found in owner/name.",
            ),
            (
                r#"[[{"author":{"type":"Bot","login":"automation"}},{"author":{"type":"User","login":"automation[bot]"}}]]"#,
                "",
            ),
            (
                r#"[[{"author":{"login":"owner"}}]]"#,
                "Blocked merge: GitHub returned malformed commit data for owner/name.",
            ),
        ] {
            let github = FakeGitHubCli::new([
                Ok("owner/name\n"),
                Ok("owner\n"),
                Ok(owner_page()),
                Ok(commits),
            ]);
            let result = guard("git merge topic", Path::new("/workspace"), &github);
            if expected.is_empty() {
                assert_eq!(result, Ok(()), "{commits}");
            } else {
                assert_eq!(result, Err(expected.to_owned()), "{commits}");
            }
        }
    }

    #[test]
    fn blocks_human_anonymous_and_malformed_contributors() {
        for (contributors, expected) in [
            (
                r#"[[{"type":"User","login":"other"}]]"#,
                "Blocked merge: owner/name has human contributor other.",
            ),
            (
                r#"[[{"type":"User","login":null}]]"#,
                "Blocked merge: anonymous contributor found in owner/name.",
            ),
            (
                r#"[[{"type":"User"}]]"#,
                "Blocked merge: anonymous contributor found in owner/name.",
            ),
            (
                r#"[[{"login":"owner"}]]"#,
                "Blocked merge: GitHub returned malformed contributor data for owner/name.",
            ),
            (
                r#"[[{"type":5,"login":"owner"}]]"#,
                "Blocked merge: GitHub returned malformed contributor data for owner/name.",
            ),
            (
                "",
                "Blocked merge: GitHub returned malformed contributor data for owner/name.",
            ),
            (
                "{}",
                "Blocked merge: GitHub returned malformed contributor data for owner/name.",
            ),
        ] {
            let github = FakeGitHubCli::new([Ok("owner/name\n"), Ok("owner\n"), Ok(contributors)]);
            assert_eq!(
                guard("git merge topic", Path::new("/workspace"), &github),
                Err(expected.to_owned()),
                "{contributors}"
            );
        }
    }

    #[test]
    fn blocks_repository_identity_contributor_and_executable_failures() {
        let cases = [
            (vec![Err("repository failure")], REPOSITORY_ERROR),
            (
                vec![Ok("owner/name\n"), Err("identity failure")],
                IDENTITY_ERROR,
            ),
            (
                vec![
                    Ok("owner/name\n"),
                    Ok("owner\n"),
                    Err("contributors failure"),
                ],
                "Blocked merge: could not fetch contributors for owner/name.",
            ),
            (
                vec![
                    Ok("owner/name\n"),
                    Ok("owner\n"),
                    Ok(owner_page()),
                    Err("commits failure"),
                ],
                "Blocked merge: could not verify commit authors for owner/name.",
            ),
            (vec![Err("executable unavailable")], REPOSITORY_ERROR),
        ];
        for (responses, expected) in cases {
            let github = FakeGitHubCli::new(responses);
            assert_eq!(
                guard("git merge topic", Path::new("/workspace"), &github),
                Err(expected.to_owned())
            );
        }
    }

    #[test]
    fn rejects_missing_malformed_and_trailing_cli_arguments() {
        for arguments in [
            vec![],
            vec!["--check".to_owned()],
            vec!["--check".to_owned(), "git merge topic".to_owned()],
            vec![
                "--check".to_owned(),
                "git merge topic".to_owned(),
                "--cwd".to_owned(),
            ],
            vec![
                "--check".to_owned(),
                "git merge topic".to_owned(),
                "--cwd".to_owned(),
                "--other".to_owned(),
            ],
            vec![
                "--check".to_owned(),
                "git merge topic".to_owned(),
                "--cwd".to_owned(),
                "/workspace".to_owned(),
                "extra".to_owned(),
            ],
        ] {
            assert_eq!(parse_cli(&arguments), Err(INVALID_ARGUMENTS_ERROR));
        }
        assert_eq!(
            parse_cli(&[
                "--check".to_owned(),
                "git merge topic".to_owned(),
                "--cwd".to_owned(),
                "/workspace".to_owned(),
            ]),
            Ok(("git merge topic".to_owned(), PathBuf::from("/workspace")))
        );
    }
}
