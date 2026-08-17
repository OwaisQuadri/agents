use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use tool_sync::cli::{self, Cli, Mode};
use tool_sync::manifest;
use tool_sync::planner::{self, Context};
use tool_sync::{apply, SyncError};

fn main() -> ExitCode {
    match command(env::args_os(), home()) {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn command(
    args: impl Iterator<Item = OsString>,
    home: Result<PathBuf, SyncError>,
) -> Result<String, SyncError> {
    run(cli::parse(args, home?)?)
}

fn home() -> Result<PathBuf, SyncError> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| SyncError::Arguments("HOME is not set".to_owned()))
}

fn run(cli: Cli) -> Result<String, SyncError> {
    let manifest = manifest::load(&cli.manifest_path)?;
    let context = Context {
        repository_root: cli.repository_root,
        cache_root: cli.home_root.join(".cache/tool-sync"),
        home_root: cli.home_root,
        platform: cli.platform,
    };
    let plan = planner::build(&manifest, &context)?;
    let report = apply::render(&plan, cli.is_dry_run);
    if cli.mode == Mode::Apply {
        apply::run(&plan, cli.is_dry_run)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let root =
                env::temp_dir().join(format!("tool-sync-command-{}-{id}", std::process::id()));
            fs::create_dir_all(root.join("source")).expect("source directory");
            Self { root }
        }

        fn write_manifest(&self, installer: &Path) -> PathBuf {
            let manifest = self.root.join("tools.toml");
            fs::write(
                &manifest,
                format!(
                    r#"[[tools]]
name = "demo"
platforms = ["linux"]
commands = ["demo"]
source = {{ path = "source" }}
installer = {{ command = {:?}, args = ["apply"], preview_args = ["preview"] }}
"#,
                    installer.display().to_string()
                ),
            )
            .expect("manifest");
            manifest
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn command_parses_plans_and_checks_without_writing() {
        let fixture = Fixture::new();
        let marker = fixture.root.join("installer-ran");
        let installer = fixture.root.join("installer.sh");
        fs::write(
            &installer,
            format!("#!/bin/sh\ntouch {:?}\n", marker.display().to_string()),
        )
        .expect("installer");
        let manifest = fixture.write_manifest(&installer);
        let home = fixture.root.join("home");
        fs::create_dir(&home).expect("home");
        let arguments = [
            OsString::from("tool-sync"),
            OsString::from("--manifest"),
            manifest.into_os_string(),
            OsString::from("--repository-root"),
            fixture.root.clone().into_os_string(),
            OsString::from("--home"),
            home.clone().into_os_string(),
            OsString::from("--platform"),
            OsString::from("linux"),
            OsString::from("--check"),
        ];

        let report = command(arguments.into_iter(), Ok(home)).expect("command succeeds");

        assert!(report.contains("install demo"), "{report}");
        assert!(report.contains("link command"), "{report}");
        assert!(!marker.exists());
    }

    #[test]
    fn command_propagates_manifest_errors() {
        let fixture = Fixture::new();
        let home = fixture.root.join("home");
        fs::create_dir(&home).expect("home");
        let arguments = [
            OsString::from("tool-sync"),
            OsString::from("--manifest"),
            fixture.root.join("missing.toml").into_os_string(),
            OsString::from("--repository-root"),
            fixture.root.clone().into_os_string(),
            OsString::from("--home"),
            home.clone().into_os_string(),
        ];

        assert!(command(arguments.into_iter(), Ok(home)).is_err());
    }
}
