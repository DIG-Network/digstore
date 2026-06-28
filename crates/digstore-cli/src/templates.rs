//! Built-in project templates for `digstore new`.
//!
//! Each template is a working LOCAL project — a `dig.toml`, a starter app, and
//! (for the dapp/NFT templates) a `window.chia` usage example — scaffolded with
//! NO wallet, NO chain, and NO spend. The files are embedded in the binary at
//! build time via `include_str!`, so `digstore new` works from a single binary
//! with no network or template-fetch step.
//!
//! "See it work before you pay": scaffold → `digstore dev` (free local preview)
//! → `digstore deploy` (the only step that spends DIG).

/// One embedded file in a template: its path RELATIVE to the project root, and
/// its verbatim contents.
pub struct TemplateFile {
    pub path: &'static str,
    pub contents: &'static str,
}

/// A named, self-contained project template.
pub struct Template {
    /// The name passed to `digstore new <name>`.
    pub name: &'static str,
    /// A one-line, task-first description (shown by `--list`).
    pub description: &'static str,
    /// The files written into the target directory, in order.
    pub files: &'static [TemplateFile],
}

/// `include_str!` a template file relative to `templates/<dir>/` and bind it to
/// its destination path within the scaffolded project.
macro_rules! tf {
    ($dir:literal, $path:literal) => {
        TemplateFile {
            path: $path,
            contents: include_str!(concat!("../templates/", $dir, "/", $path)),
        }
    };
}

static STATIC_SITE: &[TemplateFile] = &[
    tf!("static-site", "dig.toml"),
    tf!("static-site", "index.html"),
    tf!("static-site", "style.css"),
];

static VITE_REACT: &[TemplateFile] = &[
    tf!("vite-react", "dig.toml"),
    tf!("vite-react", "package.json"),
    tf!("vite-react", "vite.config.js"),
    tf!("vite-react", "index.html"),
    tf!("vite-react", ".gitignore"),
    tf!("vite-react", "src/main.jsx"),
    tf!("vite-react", "src/App.jsx"),
];

static NEXT_STATIC: &[TemplateFile] = &[
    tf!("next-static", "dig.toml"),
    tf!("next-static", "package.json"),
    tf!("next-static", "next.config.mjs"),
    tf!("next-static", ".gitignore"),
    tf!("next-static", "app/layout.jsx"),
    tf!("next-static", "app/page.jsx"),
];

static NFT_DROP: &[TemplateFile] = &[
    tf!("nft-drop", "dig.toml"),
    tf!("nft-drop", "index.html"),
    tf!("nft-drop", "app.js"),
    tf!("nft-drop", "style.css"),
];

static DAPP_WINDOW_CHIA: &[TemplateFile] = &[
    tf!("dapp-window-chia", "dig.toml"),
    tf!("dapp-window-chia", "index.html"),
    tf!("dapp-window-chia", "app.js"),
    tf!("dapp-window-chia", "style.css"),
];

/// Every built-in template, in display order.
pub static TEMPLATES: &[Template] = &[
    Template {
        name: "static-site",
        description: "a plain HTML/CSS site (no build step)",
        files: STATIC_SITE,
    },
    Template {
        name: "vite-react",
        description: "a Vite + React app (window.chia wired)",
        files: VITE_REACT,
    },
    Template {
        name: "next-static",
        description: "a statically-exported Next.js app",
        files: NEXT_STATIC,
    },
    Template {
        name: "nft-drop",
        description: "an NFT drop / mint page",
        files: NFT_DROP,
    },
    Template {
        name: "dapp-window-chia",
        description: "a minimal dapp using the window.chia wallet",
        files: DAPP_WINDOW_CHIA,
    },
];

/// Look a template up by name.
pub fn find(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.name == name)
}

/// The available template names, comma-joined (for error messages).
pub fn names() -> String {
    TEMPLATES
        .iter()
        .map(|t| t.name)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_five_templates_exist_with_unique_names() {
        let names: Vec<&str> = TEMPLATES.iter().map(|t| t.name).collect();
        for expected in [
            "static-site",
            "vite-react",
            "next-static",
            "nft-drop",
            "dapp-window-chia",
        ] {
            assert!(names.contains(&expected), "missing template {expected}");
        }
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "template names must be unique");
    }

    /// Every template MUST ship a dig.toml (so `dev`/`deploy` have config) and a
    /// non-empty starter — the scaffolded project must be runnable as-is.
    #[test]
    fn every_template_has_dig_toml_and_nonempty_files() {
        for t in TEMPLATES {
            assert!(
                t.files.iter().any(|f| f.path == "dig.toml"),
                "{} has no dig.toml",
                t.name
            );
            assert!(!t.files.is_empty(), "{} has no files", t.name);
            for f in t.files {
                assert!(!f.contents.is_empty(), "{}: {} is empty", t.name, f.path);
            }
        }
    }

    /// The dapp + NFT templates demonstrate the wallet API: they must reference
    /// `window.chia` somewhere so the scaffold is a real usage example.
    #[test]
    fn wallet_templates_reference_window_chia() {
        for name in ["dapp-window-chia", "nft-drop"] {
            let t = find(name).unwrap();
            let mentions = t.files.iter().any(|f| f.contents.contains("window.chia"));
            assert!(mentions, "{name} should demonstrate window.chia");
        }
    }

    #[test]
    fn find_unknown_is_none() {
        assert!(find("nope").is_none());
    }
}
