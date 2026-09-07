use std::fs;
use std::os::unix::fs::{symlink, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use tool_sync::manifest::{self, ToolSource};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

fn fixture_root() -> PathBuf {
    loop {
        let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "tool-sync-donsetch-migration-{}-{id}",
            std::process::id()
        ));
        match fs::create_dir(&root) {
            Ok(()) => return root,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => panic!("create fixture {}: {error}", root.display()),
        }
    }
}

fn installer() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../install.sh"))
        .expect("read tracked installer")
}

fn helper(source: &str, name: &str) -> String {
    let body = source
        .split_once(&format!("{name}() {{"))
        .expect("installer helper")
        .1
        .split_once("\n}")
        .expect("helper end")
        .0;
    format!("{name}() {{{body}\n}}\n")
}

fn run_script(home: &Path, script: &str) -> String {
    let output = Command::new("/bin/zsh")
        .args(["-fc", script])
        .env("HOME", home)
        .env("HOME_TARGET", home)
        .current_dir(home)
        .output()
        .expect("run installer fragment in Z shell");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("installer output")
}

#[test]
fn manifest_keeps_both_web_packages_and_independent_video() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/tools.toml");
    let manifest = manifest::load(&path).expect("load tracked manifest");
    for (name, package, repository) in [
        ("donsetch", "npm", "donsetch"),
        ("web", ".", "nicobailon/pi-web-access"),
    ] {
        let entries = manifest
            .tools
            .iter()
            .filter(|tool| tool.name == name)
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 1, "one {name} package");
        assert_eq!(entries[0].pi_package.as_deref(), Some(Path::new(package)));
        let ToolSource::Git { url, .. } = &entries[0].source else {
            panic!("{name} must retain its Git source");
        };
        assert!(url.to_lowercase().contains(repository));
    }
    assert!(manifest
        .tools
        .iter()
        .any(|tool| tool.name == "transcript-directed-video-processor"));
}

#[test]
fn generated_grants_are_separate_yaml_items_with_both_routes() {
    let source = installer();
    let program = source
        .split_once("gen=\"$(awk '")
        .expect("agent generator")
        .1
        .split_once("' \"$src\")\"")
        .expect("generator end")
        .0;
    let root = fixture_root();
    let agent = root.join("agent.md");
    fs::write(&agent, "---\nname: fixture\ndescription: fixture\ntools: Read, WebFetch, WebSearch, Bash\n---\nfixture body\n").expect("agent fixture");
    let output = Command::new("/bin/zsh")
        .args(["-fc", "set -euo pipefail; awk \"$PROGRAM\" \"$SOURCE\""])
        .env("PROGRAM", program)
        .env("SOURCE", &agent)
        .current_dir(&root)
        .output()
        .expect("execute tracked generator");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let generated = String::from_utf8(output.stdout).expect("generated text");
    let grants = generated
        .split_once("tools:\n")
        .expect("tool grants")
        .1
        .split_once("---\n")
        .expect("frontmatter end")
        .0
        .lines()
        .map(|line| line.strip_prefix("  - ").expect("separate list item"))
        .collect::<Vec<_>>();
    assert_eq!(
        grants,
        [
            "read",
            "ext:pi-extension/web_fetch",
            "ext:pi-extension/web_crawl",
            "ext:web/fetch_content",
            "ext:web/source_check",
            "ext:web/get_search_content",
            "ext:pi-extension/web_search",
            "ext:web/fallback_web_search",
            "bash",
        ]
    );
    assert_eq!(
        grants
            .iter()
            .copied()
            .filter(|grant| !grant.starts_with("ext:"))
            .collect::<Vec<_>>(),
        ["read", "bash"]
    );
    assert!(generated.starts_with("---\nname: \"fixture\"\ndescription: \"fixture\"\ntools:\n"));
    assert!(generated.contains("---\nfixture body\n"));
    assert!(generated.contains("Use DonSeTch"));
    assert!(generated.contains("unsupported"));
    assert!(generated.contains("Report why"));
    fs::write(
        &agent,
        "---\nname: local\ndescription: local\ntools: Read\n---\nlocal body\n",
    )
    .expect("local agent");
    let output = Command::new("/bin/zsh")
        .args(["-fc", "awk \"$PROGRAM\" \"$SOURCE\""])
        .env("PROGRAM", program)
        .env("SOURCE", &agent)
        .output()
        .expect("local generation");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("local text"),
        "---\nname: \"local\"\ndescription: \"local\"\ntools:\n  - read\n---\nlocal body\n"
    );
}

fn merge_config(home: &Path) {
    let source = installer();
    let block = source
        .split_once("  PI_WEBSEARCH=")
        .expect("web configuration")
        .1
        .split_once("\nfi")
        .expect("web configuration end")
        .0;
    let script = format!(
        "set -euo pipefail\nIS_DRY=0\nplan() {{ :; }}\nbackup() {{ :; }}\n{}{}  PI_WEBSEARCH={block}\n",
        helper(&source, "run"), helper(&source, "json_update")
    );
    run_script(home, &script);
}

#[test]
fn merged_config_preserves_settings_and_repeat_application() {
    let root = fixture_root();
    fs::create_dir(root.join(".pi")).expect("config directory");
    let path = root.join(".pi/web-search.json");
    fs::write(&path, r#"{"workflow":"curator","autoOpenBrowser":true,"provider":"custom","toolNames":{"webSearch":"web_search","fetchContent":"custom_fetch"},"tools":{"webSearch":{"enabled":false,"limit":7},"fetchContent":{"enabled":false}},"authFetch":{"profiles":{"saved":{"browser":"chrome"}}}}"#).expect("existing preferences");
    merge_config(&root);
    let first = fs::read(&path).expect("merged preferences");
    let result = run_script(&root, "jq -e '. == {workflow: \"none\", autoOpenBrowser: false, provider: \"custom\", toolNames: {webSearch: \"fallback_web_search\", fetchContent: \"custom_fetch\"}, tools: {webSearch: {enabled: true, limit: 7}, fetchContent: {enabled: false}}, authFetch: {profiles: {saved: {browser: \"chrome\"}}}}' .pi/web-search.json");
    assert_eq!(result.trim(), "true");
    merge_config(&root);
    assert_eq!(fs::read(&path).expect("second merge"), first);
}

#[test]
fn clean_config_enables_distinct_fallback_and_disables_curator() {
    let root = fixture_root();
    merge_config(&root);
    assert_eq!(run_script(&root, "jq -e '. == {workflow: \"none\", autoOpenBrowser: false, toolNames: {webSearch: \"fallback_web_search\"}, tools: {webSearch: {enabled: true}}}' .pi/web-search.json").trim(), "true");
    let first = fs::read(root.join(".pi/web-search.json")).expect("clean config");
    merge_config(&root);
    assert_eq!(
        fs::read(root.join(".pi/web-search.json")).expect("repeat config"),
        first
    );
}

#[test]
fn obsolete_loop_does_not_retire_either_web_link() {
    let root = fixture_root();
    let extensions = root.join(".pi/agent/extensions");
    let target = root.join("checkout");
    fs::create_dir_all(&extensions).expect("extensions");
    fs::create_dir(&target).expect("checkout");
    fs::write(target.join("index.ts"), "export {};\n").expect("package content");
    let mut identities = Vec::new();
    for name in ["web", "npm"] {
        symlink(&target, extensions.join(name)).expect("extension link");
        let identity = fs::symlink_metadata(extensions.join(name)).expect("link identity");
        identities.push((identity.dev(), identity.ino()));
    }
    let source = installer();
    let obsolete = source
        .split_once("for obsolete in \\\n")
        .expect("obsolete loop")
        .1
        .split_once("\ndone")
        .expect("obsolete loop end")
        .0;
    let script = format!("set -euo pipefail\nsetopt NULL_GLOB\nIS_DRY=0\nSTAMP=fixture\nplan() {{ :; }}\n{}{}for obsolete in \\\n{obsolete}\ndone\n", helper(&source, "run"), helper(&source, "retire_pi_extension"));
    run_script(&root, &script);
    run_script(&root, &script);
    for (name, identity) in ["web", "npm"].into_iter().zip(identities) {
        assert_eq!(
            fs::read_link(extensions.join(name)).expect("retained link"),
            target
        );
        let retained = fs::symlink_metadata(extensions.join(name)).expect("retained identity");
        assert_eq!(identity, (retained.dev(), retained.ino()));
    }
    assert!(!root.join(".pi/agent/retired-extensions").exists());
    assert_eq!(
        fs::read(target.join("index.ts")).expect("package retained"),
        b"export {};\n"
    );
}
