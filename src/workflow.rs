use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenKind {
    Image,
    Project,
}

pub fn classify_open_path(path: &Path) -> OpenKind {
    if path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("threshiator"))
    {
        OpenKind::Project
    } else {
        OpenKind::Image
    }
}

pub fn ensure_project_extension(path: PathBuf) -> PathBuf {
    if classify_open_path(&path) == OpenKind::Project {
        path
    } else {
        let mut value = path.into_os_string();
        value.push(".threshiator");
        value.into()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplacementDecision {
    ReplaceNow,
    SaveThenReplace,
    KeepCurrent,
}

pub fn replacement_decision(dirty: bool, response: &str) -> ReplacementDecision {
    if !dirty {
        ReplacementDecision::ReplaceNow
    } else {
        match response {
            "save" => ReplacementDecision::SaveThenReplace,
            "discard" => ReplacementDecision::ReplaceNow,
            _ => ReplacementDecision::KeepCurrent,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveResolution {
    CurrentSuccess,
    Failed,
    Cancelled,
}

pub fn resolve_pending_after_save<T>(
    pending: &mut Option<T>,
    resolution: SaveResolution,
) -> Option<T> {
    match resolution {
        SaveResolution::CurrentSuccess => pending.take(),
        SaveResolution::Failed | SaveResolution::Cancelled => {
            pending.take();
            None
        }
    }
}
