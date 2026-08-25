use std::collections::BTreeSet;

use serde::Serialize;

use crate::capture::Baseline;

#[derive(Debug, Serialize)]
pub struct RefMove {
    pub name: String,
    pub old: Option<String>,
    pub new: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct Delta {
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub deleted: Vec<String>,
    pub untracked: Vec<String>,
    pub moved_refs: Vec<RefMove>,
}

impl Delta {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.modified.is_empty()
            && self.deleted.is_empty()
            && self.untracked.is_empty()
            && self.moved_refs.is_empty()
    }
}

/// Diffs a recapture against a stamp and returns only what moved between them.
///
/// Takes the stamped baseline and the fresh capture. Returns the delta, where a path
/// carrying the same status code in both captures is absent and a ref whose hash differs
/// lands in `moved_refs` rather than in any file category.
pub fn diff(stamp: &Baseline, fresh: &Baseline) -> Delta {
    let mut delta = Delta::default();
    let paths: BTreeSet<&String> = stamp.entries.keys().chain(fresh.entries.keys()).collect();
    for path in paths {
        let before = stamp.entries.get(path);
        let after = fresh.entries.get(path);
        if before == after {
            continue;
        }
        let status = after.and_then(|entry| entry.get(..2)).unwrap_or_default();
        match after {
            None => delta.modified.push(path.clone()),
            Some(_) if status == "??" && before.is_none() => delta.untracked.push(path.clone()),
            Some(_) if status == "??" => delta.modified.push(path.clone()),
            Some(_) if status.contains('D') => delta.deleted.push(path.clone()),
            Some(_) if status.contains('A') => delta.added.push(path.clone()),
            Some(_) => delta.modified.push(path.clone()),
        }
    }

    let names: BTreeSet<&String> = stamp.refs.keys().chain(fresh.refs.keys()).collect();
    for name in names {
        let old = stamp.refs.get(name);
        let new = fresh.refs.get(name);
        if old == new {
            continue;
        }
        delta.moved_refs.push(RefMove {
            name: name.clone(),
            old: old.cloned(),
            new: new.cloned(),
        });
    }
    delta
}

pub fn render(delta: &Delta) -> String {
    if delta.is_empty() {
        return "delta empty: this run changed nothing since the stamp\n".to_string();
    }
    let mut out = String::new();
    for (label, paths) in [
        ("added", &delta.added),
        ("modified", &delta.modified),
        ("deleted", &delta.deleted),
        ("untracked-since-stamp", &delta.untracked),
    ] {
        for path in paths {
            out.push_str(&format!("{label}: {path}\n"));
        }
    }
    for moved in &delta.moved_refs {
        let old = moved.old.as_deref().unwrap_or("(absent)");
        let new = moved.new.as_deref().unwrap_or("(absent)");
        out.push_str(&format!("moved ref: {} {} -> {}\n", moved.name, old, new));
    }
    out
}
