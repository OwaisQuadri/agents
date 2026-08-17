use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::manifest::Platform;
use crate::SyncError;

/// Selects whether synchronization applies the plan or only checks it.
/// It takes no inputs, reports the requested command mode, and cannot fail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Apply,
    Check,
}

/// Holds validated command inputs for tool synchronization.
/// It contains manifest and root paths, platform, mode, and dry-run state; constructing it through `parse` can return `SyncError`.
#[derive(Debug, Eq, PartialEq)]
pub struct Cli {
    pub manifest_path: PathBuf,
    pub repository_root: PathBuf,
    pub home_root: PathBuf,
    pub platform: Platform,
    pub mode: Mode,
    pub is_dry_run: bool,
}

/// Parses process arguments into validated synchronization inputs.
/// It takes an iterator beginning with argv0 and an absolute default home, and returns a `Cli`.
/// It returns `SyncError` for a missing argv0 or value, unknown argument, unsupported platform, or relative home.
pub fn parse(args: impl Iterator<Item = OsString>, home: PathBuf) -> Result<Cli, SyncError> {
    if !home.is_absolute() {
        return Err(argument_error(format!(
            "home must be absolute: {}",
            home.display()
        )));
    }

    let mut args = args;
    args.next()
        .ok_or_else(|| argument_error("program name is missing"))?;

    let mut repository_root = None;
    let mut selected_home = home;
    let mut manifest_path = None;
    let mut platform = host_platform()?;
    let mut mode = Mode::Apply;
    let mut is_dry_run = false;

    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("--manifest") => manifest_path = Some(path_value("--manifest", &mut args)?),
            Some("--repository-root") => {
                repository_root = Some(path_value("--repository-root", &mut args)?)
            }
            Some("--home") => selected_home = path_value("--home", &mut args)?,
            Some("--platform") => {
                let value = value("--platform", &mut args)?;
                platform = parse_platform(&value)?;
            }
            Some("--dry-run") => is_dry_run = true,
            Some("--check") => mode = Mode::Check,
            _ => return Err(argument_error(format!("unknown argument {:?}", argument))),
        }
    }

    if !selected_home.is_absolute() {
        return Err(argument_error(format!(
            "home must be absolute: {}",
            selected_home.display()
        )));
    }
    let repository_root = repository_root.unwrap_or_else(|| selected_home.join("Documents/agents"));
    if !repository_root.is_absolute() {
        return Err(argument_error(format!(
            "repository root must be absolute: {}",
            repository_root.display()
        )));
    }

    let manifest_path = manifest_path
        .map(|path| absolute_from(&repository_root, path))
        .unwrap_or_else(|| repository_root.join("config/tools.toml"));

    Ok(Cli {
        manifest_path,
        repository_root,
        home_root: selected_home,
        platform,
        mode,
        is_dry_run,
    })
}

fn value(flag: &str, args: &mut impl Iterator<Item = OsString>) -> Result<OsString, SyncError> {
    args.next()
        .ok_or_else(|| argument_error(format!("{flag} requires a value")))
}

fn path_value(flag: &str, args: &mut impl Iterator<Item = OsString>) -> Result<PathBuf, SyncError> {
    value(flag, args).map(PathBuf::from)
}

fn absolute_from(root: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn parse_platform(value: &OsStr) -> Result<Platform, SyncError> {
    match value.to_str() {
        Some("macos") => Ok(Platform::Macos),
        Some("linux") => Ok(Platform::Linux),
        _ => Err(argument_error(format!(
            "invalid platform {:?}; expected macos or linux",
            value
        ))),
    }
}

fn host_platform() -> Result<Platform, SyncError> {
    match std::env::consts::OS {
        "macos" => Ok(Platform::Macos),
        "linux" => Ok(Platform::Linux),
        platform => Err(argument_error(format!(
            "platform {platform:?} is unsupported; pass --platform macos or linux"
        ))),
    }
}

fn argument_error(detail: impl Into<String>) -> SyncError {
    SyncError::Arguments(detail.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_tokens(tokens: &[&str]) -> Result<Cli, SyncError> {
        parse(
            tokens.iter().map(OsString::from),
            PathBuf::from("/home/tester"),
        )
    }

    #[test]
    fn parses_every_supported_input() {
        let cli = parse_tokens(&[
            "tool-sync",
            "--repository-root",
            "/repo",
            "--manifest",
            "fixtures/tools.toml",
            "--home",
            "/tmp/home",
            "--platform",
            "linux",
            "--dry-run",
            "--check",
        ])
        .expect("arguments parse");

        assert_eq!(cli.repository_root, Path::new("/repo"));
        assert_eq!(cli.manifest_path, Path::new("/repo/fixtures/tools.toml"));
        assert_eq!(cli.home_root, Path::new("/tmp/home"));
        assert_eq!(cli.platform, Platform::Linux);
        assert_eq!(cli.mode, Mode::Check);
        assert!(cli.is_dry_run);
    }

    #[test]
    fn derives_defaults_from_the_selected_absolute_home() {
        let cli = parse_tokens(&["tool-sync", "--home", "/home/other"]).expect("defaults parse");

        assert_eq!(
            cli.repository_root,
            Path::new("/home/other/Documents/agents")
        );
        assert_eq!(
            cli.manifest_path,
            Path::new("/home/other/Documents/agents/config/tools.toml")
        );
        assert_eq!(cli.mode, Mode::Apply);
        assert!(!cli.is_dry_run);
    }

    #[test]
    fn rejects_bad_arguments_and_relative_homes() {
        for tokens in [
            &["tool-sync", "--unknown"][..],
            &["tool-sync", "--manifest"][..],
            &["tool-sync", "--platform", "windows"][..],
            &["tool-sync", "--home", "relative"][..],
        ] {
            assert!(
                parse_tokens(tokens).is_err(),
                "unexpectedly parsed {tokens:?}"
            );
        }
        assert!(parse(
            [OsString::from("tool-sync")].into_iter(),
            PathBuf::from("relative")
        )
        .is_err());
    }
}
