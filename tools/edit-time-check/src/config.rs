use crate::Rule;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    version: u8,
    rules: Option<Vec<Rule>>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    generated: Option<Vec<String>>,
    total_ms: Option<u64>,
    process_budget_ms: Option<u64>,
    rule_ms: Option<BTreeMap<Rule, u64>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Config {
    pub rules: Vec<Rule>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub generated: Vec<String>,
    pub total_ms: u64,
    pub process_budget_ms: u64,
    pub rule_ms: BTreeMap<Rule, u64>,
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut config = Self {
            rules: vec![
                Rule::Privacy,
                Rule::CommentLength,
                Rule::CommentShape,
                Rule::BooleanName,
            ],
            include: vec!["**".into()],
            exclude: [
                ".git",
                "node_modules",
                "vendor",
                "target",
                "build",
                "dist",
                ".cache",
                "__pycache__",
                ".venv",
            ]
            .map(|p| format!("**/{p}/**"))
            .into(),
            generated: vec![
                "**/*.generated.*".into(),
                "**/*.min.js".into(),
                "**/*.g.cs".into(),
            ],
            total_ms: 20_000,
            process_budget_ms: 3000,
            rule_ms: BTreeMap::new(),
        };
        config.apply(text)?;
        Ok(config)
    }

    fn apply(&mut self, text: &str) -> Result<(), String> {
        let file: FileConfig =
            toml::from_str(text).map_err(|_| "invalid configuration fields or TOML")?;
        if file.version != 1 {
            return Err("unsupported configuration version".into());
        }
        if let Some(rules) = file.rules {
            for (index, rule) in rules.iter().enumerate() {
                if rules[..index].contains(rule) {
                    return Err("duplicate rule identifier".into());
                }
            }
            self.rules = rules;
        }
        for (destination, value) in [
            (&mut self.include, file.include),
            (&mut self.exclude, file.exclude),
            (&mut self.generated, file.generated),
        ] {
            if let Some(patterns) = value {
                *destination = patterns;
            }
        }
        if let Some(total) = file.total_ms {
            if total == 0 || total > self.total_ms {
                return Err(
                    "total_ms must be positive and cannot raise the inherited ceiling".into(),
                );
            }
            self.total_ms = total;
        }
        if let Some(process) = file.process_budget_ms {
            if process == 0 || process > self.process_budget_ms {
                return Err(
                    "process_budget_ms must be positive and cannot raise the inherited ceiling"
                        .into(),
                );
            }
            self.process_budget_ms = process;
        }
        if let Some(limits) = file.rule_ms {
            for (rule, limit) in limits {
                if limit == 0 || limit > self.rule_limit(rule) {
                    return Err(
                        "rule_ms must be positive and cannot raise the inherited ceiling".into(),
                    );
                }
                self.rule_ms.insert(rule, limit);
            }
        }
        self.selection()?;
        Ok(())
    }

    fn apply_repository(&mut self, text: &str) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.apply(text)?;
        if !self.rules.iter().all(|rule| candidate.rules.contains(rule)) {
            return Err("repository configuration cannot remove inherited rules".into());
        }
        if !self
            .include
            .iter()
            .all(|pattern| candidate.include.contains(pattern))
        {
            return Err("repository include must retain every inherited pattern".into());
        }
        for (name, inherited, proposed) in [
            ("exclude", &self.exclude, &candidate.exclude),
            ("generated", &self.generated, &candidate.generated),
        ] {
            if !proposed.iter().all(|pattern| inherited.contains(pattern)) {
                return Err(format!(
                    "repository {name} cannot add patterns outside the inherited list"
                ));
            }
        }
        *self = candidate;
        Ok(())
    }

    pub fn rule_limit(&self, rule: Rule) -> u64 {
        self.rule_ms.get(&rule).copied().unwrap_or(500)
    }

    fn selection(&self) -> Result<(GlobSet, GlobSet, GlobSet), String> {
        Ok((
            patterns(&self.include)?,
            patterns(&self.exclude)?,
            patterns(&self.generated)?,
        ))
    }

    pub fn is_selected(&self, path: &str) -> Result<bool, String> {
        let (include, exclude, generated) = self.selection()?;
        Ok(include.is_match(path) && !exclude.is_match(path) && !generated.is_match(path))
    }
}

fn patterns(patterns: &[String]) -> Result<GlobSet, String> {
    if patterns.len() > 256 {
        return Err("too many selection patterns".into());
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        if pattern.is_empty() || pattern.len() > 1024 || pattern.contains('\0') {
            return Err("invalid selection pattern".into());
        }
        builder.add(
            GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()
                .map_err(|_| "invalid selection glob")?,
        );
    }
    builder
        .build()
        .map_err(|_| "cannot compile selection patterns".into())
}

#[derive(Clone, Debug)]
pub struct Options {
    pub config: PathBuf,
    pub rule_document: PathBuf,
    pub code_style: PathBuf,
    pub identifiers: Option<PathBuf>,
    pub judgment_configuration: String,
}

impl Options {
    pub fn for_paths(config: PathBuf, rule_document: PathBuf, code_style: PathBuf) -> Self {
        Self {
            config,
            rule_document,
            code_style,
            identifiers: None,
            judgment_configuration: String::new(),
        }
    }

    pub fn from_args(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut values = BTreeMap::new();
        let mut args = args;
        while let Some(key) = args.next() {
            if !matches!(
                key.as_str(),
                "--config"
                    | "--rule-document"
                    | "--code-style"
                    | "--identifiers"
                    | "--judgment-configuration"
            ) || values.contains_key(&key)
            {
                return Err("unknown or duplicate argument".into());
            }
            values.insert(
                key,
                args.next()
                    .filter(|v| !v.is_empty())
                    .ok_or("missing argument value")?,
            );
        }
        let mut options = Self::for_paths(
            values
                .remove("--config")
                .ok_or("--config is required")?
                .into(),
            values
                .remove("--rule-document")
                .ok_or("--rule-document is required")?
                .into(),
            values
                .remove("--code-style")
                .ok_or("--code-style is required")?
                .into(),
        );
        options.judgment_configuration = values
            .remove("--judgment-configuration")
            .ok_or("--judgment-configuration is required")?;
        options.identifiers = values.remove("--identifiers").map(PathBuf::from);
        Ok(options)
    }

    pub fn resolve(
        &self,
        target: &Path,
        claimed_root: Option<&str>,
    ) -> Result<(Config, String), String> {
        let parent = target.parent().ok_or("target has no parent")?;
        if !crate::is_absolute_clean(target.to_str().ok_or("target is not UTF-8")?) {
            return Err("invalid target path".into());
        }
        let existing = existing_parent(parent)?;
        let canonical = existing
            .canonicalize()
            .map_err(|_| "cannot resolve target parent")?;
        if existing != canonical {
            return Err("target parent is not canonical".into());
        }
        let root = find_repository(&canonical)?;
        if root.as_deref() != claimed_root.map(Path::new) {
            return Err("repository identity differs from target repository".into());
        }
        let mut config = Config::parse(&read_required(&self.config)?)?;
        if let Some(root) = root.as_ref() {
            let path = root.join(".edit-time.toml");
            match std::fs::symlink_metadata(&path) {
                Ok(_) => config.apply_repository(&read_required(&path)?)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("cannot inspect repository configuration".into()),
            }
        }
        let relative = target
            .strip_prefix(root.as_deref().unwrap_or(Path::new("/")))
            .map_err(|_| "cannot select target")?;
        Ok((
            config,
            relative.to_str().ok_or("target is not UTF-8")?.into(),
        ))
    }
}

fn existing_parent(parent: &Path) -> Result<&Path, String> {
    for ancestor in parent.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.is_dir() => return Ok(ancestor),
            Ok(_) => return Err("target ancestor is not a regular directory".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("cannot inspect target ancestor".into()),
        }
    }
    Err("target has no existing ancestor".into())
}

fn find_repository(parent: &Path) -> Result<Option<PathBuf>, String> {
    for ancestor in parent.ancestors() {
        let git = ancestor.join(".git");
        match std::fs::symlink_metadata(&git) {
            Ok(metadata) if metadata.is_dir() => return Ok(Some(ancestor.into())),
            Ok(metadata) if metadata.is_file() => {
                let text = read_required(&git)?;
                if !text.starts_with("gitdir: ") || text.trim_end().len() <= 8 {
                    return Err("invalid worktree repository marker".into());
                }
                return Ok(Some(ancestor.into()));
            }
            Ok(_) => return Err("unsupported repository marker".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("cannot inspect target repository".into()),
        }
    }
    Ok(None)
}

pub(crate) fn read_required(path: &Path) -> Result<String, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|_| "cannot read required configuration or rule document")?;
    if !metadata.is_file() {
        return Err("required input is not a regular file".into());
    }
    let file = std::fs::File::open(path).map_err(|_| "cannot open required input")?;
    let mut text = String::new();
    file.take(262_145)
        .read_to_string(&mut text)
        .map_err(|_| "cannot read required UTF-8 input")?;
    if text.trim().is_empty() || text.len() > 262_144 {
        return Err("required input is empty or exceeds byte limit".into());
    }
    Ok(text)
}
