//! Human + JSON rendering of command results.

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct StatusView {
    pub root: Option<String>,
    pub staged: Vec<String>,
    pub modified: Vec<String>,
    pub untracked: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct LogEntry {
    pub id: u64,
    pub root: String,
    pub timestamp: u64,
}

#[derive(Debug, Serialize)]
pub struct DiffEntry {
    pub resource_key: String,
    pub change: String, // "added" | "removed" | "modified"
}

pub fn render_status(s: &StatusView, ui: &crate::ui::Ui) {
    if ui.json() {
        ui.emit_json(s);
        return;
    }
    match &s.root {
        Some(r) => ui.line(format!("● generation root {}", &r[..r.len().min(12)])),
        None => ui.line("No commits yet"),
    }
    use crate::ui::theme::Marker;
    let group = |ui: &crate::ui::Ui, label: &str, m: Marker, items: &[String]| {
        if items.is_empty() {
            return;
        }
        ui.line(format!("{} ({})", label, items.len()));
        for it in items {
            ui.item(m, it);
        }
    };
    group(ui, "staged", Marker::Staged, &s.staged);
    group(ui, "modified", Marker::Modified, &s.modified);
    group(ui, "untracked", Marker::Untracked, &s.untracked);
    if !s.untracked.is_empty() {
        ui.hint("digstore add -A   # stage untracked files");
    }
    if s.staged.is_empty() && s.modified.is_empty() && s.untracked.is_empty() {
        ui.line("nothing to commit; working directory clean");
    }
}

pub fn render_log(entries: &[LogEntry], json: bool) -> String {
    if json {
        return serde_json::to_string_pretty(entries).expect("serialize log");
    }
    let mut out = String::new();
    for e in entries {
        out.push_str(&format!(
            "generation {}  root {}  ts {}\n",
            e.id, e.root, e.timestamp
        ));
    }
    out
}

pub fn render_diff(entries: &[DiffEntry], json: bool) -> String {
    if json {
        return serde_json::to_string_pretty(entries).expect("serialize diff");
    }
    let mut out = String::new();
    for e in entries {
        let sign = match e.change.as_str() {
            "added" => '+',
            "removed" => '-',
            _ => '~',
        };
        out.push_str(&format!("{} {}\n", sign, e.resource_key));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_status_json_has_untracked_field() {
        let s = StatusView {
            root: Some("ab".into()),
            staged: vec!["readme".into()],
            modified: vec![],
            untracked: vec!["new_file.txt".into()],
        };
        // Serialize directly to verify the JSON shape contains "untracked".
        let json = serde_json::to_string_pretty(&s).expect("serialize");
        assert!(json.contains("\"staged\""));
        assert!(json.contains("\"untracked\""));
        assert!(json.contains("readme"));
        assert!(json.contains("new_file.txt"));
    }

    #[test]
    fn status_view_serializes_all_four_fields() {
        let s = StatusView {
            root: None,
            staged: vec!["a".into()],
            modified: vec!["b".into()],
            untracked: vec!["c".into()],
        };
        let json = serde_json::to_string_pretty(&s).expect("serialize");
        assert!(json.contains("\"root\""));
        assert!(json.contains("\"staged\""));
        assert!(json.contains("\"modified\""));
        assert!(json.contains("\"untracked\""));
    }

    #[test]
    fn render_log_json_is_array() {
        let v = vec![LogEntry {
            id: 1,
            root: "aa".into(),
            timestamp: 100,
        }];
        let out = render_log(&v, true);
        assert!(out.trim_start().starts_with('['));
    }

    #[test]
    fn render_diff_human_uses_plus_for_added() {
        let v = vec![DiffEntry {
            resource_key: "b".into(),
            change: "added".into(),
        }];
        let out = render_diff(&v, false);
        assert!(out.contains("+ b"));
    }
}
