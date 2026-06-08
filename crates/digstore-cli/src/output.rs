//! Human + JSON rendering of command results.

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct StatusView {
    pub root: Option<String>,
    pub staged: Vec<String>,
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

pub fn render_status(s: &StatusView, json: bool) -> String {
    if json {
        return serde_json::to_string_pretty(s).expect("serialize status");
    }
    let mut out = String::new();
    match &s.root {
        Some(r) => out.push_str(&format!("On root {}\n", r)),
        None => out.push_str("No commits yet\n"),
    }
    if s.staged.is_empty() {
        out.push_str("nothing staged\n");
    } else {
        out.push_str("Staged for commit:\n");
        for e in &s.staged {
            out.push_str(&format!("  staged: {}\n", e));
        }
    }
    out
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
    fn render_status_json_has_staged_count() {
        let s = StatusView {
            root: Some("ab".into()),
            staged: vec!["readme".into()],
        };
        let out = render_status(&s, true);
        assert!(out.contains("\"staged\""));
        assert!(out.contains("readme"));
    }

    #[test]
    fn render_status_human_lists_entries() {
        let s = StatusView {
            root: None,
            staged: vec!["a".into(), "b".into()],
        };
        let out = render_status(&s, false);
        assert!(out.contains("a"));
        assert!(out.contains("b"));
        assert!(out.to_lowercase().contains("staged"));
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
