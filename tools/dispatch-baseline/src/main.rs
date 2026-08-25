mod capture;
mod delta;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage:
  dispatch-baseline stamp --repo <path> [--out <file>]
  dispatch-baseline check --repo <path> --stamp <file> [--json]
";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("dispatch-baseline: {message}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let subcommand = args.first().ok_or(USAGE)?.as_str();
    let options = Options::parse(&args[1..])?;
    let repo = options.repo.ok_or("--repo is required")?;
    match subcommand {
        "stamp" => {
            let baseline = capture::capture(&repo)?;
            let json =
                serde_json::to_string_pretty(&baseline).map_err(|error| error.to_string())? + "\n";
            match options.out {
                Some(path) => {
                    if output_is_inside_repo(&path, &baseline.repo)? {
                        return Err(format!(
                            "stamp output {} must be outside repository {}",
                            path.display(),
                            baseline.repo
                        ));
                    }
                    std::fs::write(&path, json)
                        .map_err(|error| format!("{}: {error}", path.display()))?;
                }
                None => print!("{json}"),
            }
            Ok(ExitCode::SUCCESS)
        }
        "check" => {
            let path = options.stamp.ok_or("--stamp is required")?;
            let raw = std::fs::read_to_string(&path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let stamp: capture::Baseline = serde_json::from_str(&raw)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            if stamp.schema_version != capture::SCHEMA_VERSION {
                return Err(format!(
                    "stamp schema version {} is not {}",
                    stamp.schema_version,
                    capture::SCHEMA_VERSION
                ));
            }
            let fresh = capture::capture(&repo)?;
            if stamp.repo != fresh.repo {
                return Err(format!(
                    "stamp repository {:?} is not requested repository {:?}",
                    stamp.repo, fresh.repo
                ));
            }
            let delta = delta::diff(&stamp, &fresh);
            if options.is_json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&delta).map_err(|error| error.to_string())?
                );
            } else {
                print!("{}", delta::render(&delta));
            }
            Ok(if delta.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        other => Err(format!("unknown subcommand {other}\n{USAGE}")),
    }
}

fn output_is_inside_repo(path: &std::path::Path, repo: &str) -> Result<bool, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("current directory: {error}"))?
            .join(path)
    };
    let parent = absolute
        .parent()
        .ok_or("stamp output has no parent directory")?;
    let parent = parent
        .canonicalize()
        .map_err(|error| format!("{}: {error}", parent.display()))?;
    let repo = std::path::Path::new(repo)
        .canonicalize()
        .map_err(|error| format!("{repo}: {error}"))?;
    Ok(parent.starts_with(repo))
}

#[derive(Default)]
struct Options {
    repo: Option<PathBuf>,
    out: Option<PathBuf>,
    stamp: Option<PathBuf>,
    is_json: bool,
}

impl Options {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut options = Options::default();
        let mut rest = args.iter();
        while let Some(flag) = rest.next() {
            let mut value = || {
                rest.next()
                    .map(PathBuf::from)
                    .ok_or(format!("{flag} needs a value"))
            };
            match flag.as_str() {
                "--repo" => options.repo = Some(value()?),
                "--out" => options.out = Some(value()?),
                "--stamp" => options.stamp = Some(value()?),
                "--json" => options.is_json = true,
                other => return Err(format!("unknown flag {other}\n{USAGE}")),
            }
        }
        Ok(options)
    }
}
