//! .digignore file parser with exact .gitignore syntax compatibility

use anyhow::Result;
use glob::{Pattern, PatternError};
use std::fs;
use std::path::{Path, PathBuf};

/// A compiled pattern from .digignore file
#[derive(Debug, Clone)]
pub struct CompiledPattern {
    /// The original pattern string
    pub original: String,
    /// The compiled glob pattern
    pub pattern: Pattern,
    /// Type of pattern (normal, negation, directory-only)
    pub pattern_type: PatternType,
    /// Whether the pattern is anchored to a specific directory level
    pub is_anchored: bool,
    /// Whether this pattern only matches directories
    pub directory_only: bool,
}

/// Type of ignore pattern
#[derive(Debug, Clone, PartialEq)]
pub enum PatternType {
    /// Normal ignore pattern
    Ignore,
    /// Negation pattern (starts with !)
    Include,
}

/// Parser for .digignore files
#[derive(Debug, Clone)]
pub struct DigignoreParser {
    /// All compiled patterns from the .digignore file
    patterns: Vec<CompiledPattern>,
    /// The directory containing the .digignore file
    base_dir: PathBuf,
}

impl DigignoreParser {
    /// Create a new parser from a .digignore file
    pub fn from_file(digignore_path: &Path) -> Result<Self> {
        let content = fs::read_to_string(digignore_path)?;
        let base_dir = digignore_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Invalid .digignore file path"))?
            .to_path_buf();

        Self::from_content(&content, base_dir)
    }

    /// Create a parser from .digignore content string
    pub fn from_content(content: &str, base_dir: PathBuf) -> Result<Self> {
        let mut patterns = Vec::new();

        for (line_num, line) in content.lines().enumerate() {
            match parse_line(line) {
                Ok(Some(pattern)) => patterns.push(pattern),
                Ok(None) => {}, // Empty line or comment
                Err(e) => {
                    eprintln!(
                        "Warning: Invalid pattern on line {}: {} ({})",
                        line_num + 1,
                        line,
                        e
                    );
                },
            }
        }

        Ok(Self { patterns, base_dir })
    }

    /// Check if a file path should be ignored
    pub fn is_ignored(&self, file_path: &Path, is_dir: bool) -> bool {
        let relative_path = match file_path.strip_prefix(&self.base_dir) {
            Ok(rel) => rel,
            Err(_) => file_path, // Use full path if not under base_dir
        };

        let mut ignored = false;

        // Process patterns in order - later patterns override earlier ones
        for pattern in &self.patterns {
            if self.matches_pattern(pattern, relative_path, is_dir) {
                match pattern.pattern_type {
                    PatternType::Ignore => ignored = true,
                    PatternType::Include => ignored = false,
                }
            }
        }

        ignored
    }

    /// Check if a pattern matches a file path
    fn matches_pattern(&self, pattern: &CompiledPattern, file_path: &Path, is_dir: bool) -> bool {
        // Directory-only patterns only match directories
        if pattern.directory_only && !is_dir {
            return false;
        }

        let path_str = file_path.to_string_lossy();

        if pattern.is_anchored {
            // Anchored patterns must match from the beginning
            pattern.pattern.matches(&path_str)
        } else {
            // Non-anchored patterns can match at any level
            // Try matching the full path and each component
            if pattern.pattern.matches(&path_str) {
                return true;
            }

            // Also try matching individual path components for basename matching
            if let Some(filename) = file_path.file_name() {
                if pattern.pattern.matches(&filename.to_string_lossy()) {
                    return true;
                }
            }

            // For patterns like "*.tmp", try matching against each path segment
            for component in file_path.components() {
                if pattern
                    .pattern
                    .matches(&component.as_os_str().to_string_lossy())
                {
                    return true;
                }
            }

            false
        }
    }

    /// Get all patterns (for debugging)
    pub fn patterns(&self) -> &[CompiledPattern] {
        &self.patterns
    }

    /// Get base directory
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }
}

/// Parse a single line from .digignore file
fn parse_line(line: &str) -> Result<Option<CompiledPattern>, PatternError> {
    let line = line.trim();

    // Skip empty lines and comments
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }

    let (pattern_type, pattern_str) = if line.starts_with('!') {
        (PatternType::Include, &line[1..])
    } else {
        (PatternType::Ignore, line)
    };

    let pattern_str = pattern_str.trim();
    if pattern_str.is_empty() {
        return Ok(None);
    }

    // Check if pattern is directory-only (ends with /)
    let (directory_only, clean_pattern) = if pattern_str.ends_with('/') {
        (true, &pattern_str[..pattern_str.len() - 1])
    } else {
        (false, pattern_str)
    };

    // Check if pattern is anchored (contains / or starts with **)
    let is_anchored = clean_pattern.contains('/') || clean_pattern.starts_with("**/");

    // Normalize pattern for glob matching
    let glob_pattern = normalize_pattern(clean_pattern, is_anchored)?;

    let compiled = Pattern::new(&glob_pattern)?;

    Ok(Some(CompiledPattern {
        original: line.to_string(),
        pattern: compiled,
        pattern_type,
        is_anchored,
        directory_only,
    }))
}

/// Normalize a pattern for glob matching
fn normalize_pattern(pattern: &str, is_anchored: bool) -> Result<String, PatternError> {
    let mut normalized = pattern.to_string();

    // Handle ** patterns
    if normalized.starts_with("**/") {
        // **/ at start means match at any depth
        normalized = normalized[3..].to_string();
        if !normalized.is_empty() {
            normalized = format!("**/{}", normalized);
        } else {
            normalized = "**".to_string();
        }
    } else if normalized.contains("/**/") {
        // /**/ in middle stays as is
    } else if normalized.ends_with("/**") {
        // /** at end means match directory and all contents
    } else if !is_anchored && !normalized.starts_with('*') {
        // Non-anchored patterns without wildcards should match at any level
        normalized = format!("**/{}", normalized);
    }

    // Ensure Windows path separators are handled
    normalized = normalized.replace('\\', "/");

    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_parse_basic_patterns() -> Result<()> {
        let content = r#"
# Comments are ignored
*.tmp
build/
!important.tmp
**/cache/
node_modules/
"#;

        let temp_dir = TempDir::new()?;
        let parser = DigignoreParser::from_content(content, temp_dir.path().to_path_buf())?;

        assert_eq!(parser.patterns().len(), 5);

        // Test basic pattern
        assert!(parser.is_ignored(Path::new("test.tmp"), false));
        assert!(!parser.is_ignored(Path::new("test.txt"), false));

        // Test directory pattern
        assert!(parser.is_ignored(Path::new("build"), true));
        assert!(!parser.is_ignored(Path::new("build"), false)); // build/ only matches dirs

        // Test negation
        assert!(!parser.is_ignored(Path::new("important.tmp"), false));

        Ok(())
    }

    #[test]
    fn test_hierarchical_matching() -> Result<()> {
        let content = "**/node_modules/\n*.log";
        let temp_dir = TempDir::new()?;
        let parser = DigignoreParser::from_content(content, temp_dir.path().to_path_buf())?;

        // Should match node_modules at any level
        assert!(parser.is_ignored(Path::new("node_modules"), true));
        assert!(parser.is_ignored(Path::new("src/node_modules"), true));
        assert!(parser.is_ignored(Path::new("deep/nested/node_modules"), true));

        // Should match .log files at any level
        assert!(parser.is_ignored(Path::new("app.log"), false));
        assert!(parser.is_ignored(Path::new("logs/app.log"), false));

        Ok(())
    }

    #[test]
    fn test_negation_patterns() -> Result<()> {
        let content = r#"
*.tmp
!important.tmp
logs/
!logs/keep.log
"#;

        let temp_dir = TempDir::new()?;
        let parser = DigignoreParser::from_content(content, temp_dir.path().to_path_buf())?;

        // Normal .tmp files should be ignored
        assert!(parser.is_ignored(Path::new("temp.tmp"), false));
        assert!(parser.is_ignored(Path::new("cache.tmp"), false));

        // But important.tmp should be included
        assert!(!parser.is_ignored(Path::new("important.tmp"), false));

        // logs directory should be ignored
        assert!(parser.is_ignored(Path::new("logs"), true));

        // But logs/keep.log should be included
        assert!(!parser.is_ignored(Path::new("logs/keep.log"), false));

        Ok(())
    }

    #[test]
    fn test_from_file() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let digignore_path = temp_dir.path().join(".digignore");

        fs::write(&digignore_path, "*.tmp\nbuild/\n")?;

        let parser = DigignoreParser::from_file(&digignore_path)?;

        assert!(parser.is_ignored(Path::new("test.tmp"), false));
        assert!(parser.is_ignored(Path::new("build"), true));
        assert!(!parser.is_ignored(Path::new("test.txt"), false));

        Ok(())
    }
}
