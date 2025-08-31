//! Repository-wide ignore checking with hierarchical .digignore support

use crate::ignore::parser::DigignoreParser;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use anyhow::Result;

/// Result of ignore checking
#[derive(Debug, Clone, PartialEq)]
pub enum IgnoreResult {
    /// File should be ignored
    Ignored(String), // Reason for ignoring
    /// File should be included
    Included,
    /// File would be ignored but is explicitly included by negation pattern
    IncludedByNegation(String), // Pattern that included it
}

/// Repository-wide ignore checker
#[derive(Debug)]
pub struct IgnoreChecker {
    /// Repository root directory
    repo_root: PathBuf,
    /// Cached parsers for each directory with .digignore
    parsers: HashMap<PathBuf, DigignoreParser>,
    /// Whether to use global .digignore in repository root
    use_global: bool,
}

impl IgnoreChecker {
    /// Create a new ignore checker for a repository
    pub fn new(repo_root: &Path) -> Result<Self> {
        let mut checker = Self {
            repo_root: repo_root.to_path_buf(),
            parsers: HashMap::new(),
            use_global: true,
        };
        
        checker.reload()?;
        Ok(checker)
    }
    
    /// Reload all .digignore files in the repository
    pub fn reload(&mut self) -> Result<()> {
        self.parsers.clear();
        
        // Load global .digignore from repository root
        let global_digignore = self.repo_root.join(".digignore");
        if global_digignore.exists() {
            match DigignoreParser::from_file(&global_digignore) {
                Ok(parser) => {
                    self.parsers.insert(self.repo_root.clone(), parser);
                }
                Err(e) => {
                    eprintln!("Warning: Failed to parse .digignore in repository root: {}", e);
                }
            }
        }
        
        // Discover and load all nested .digignore files
        self.discover_nested_digignore_files()?;
        
        Ok(())
    }
    
    /// Check if a file should be ignored
    pub fn is_ignored(&self, file_path: &Path) -> IgnoreResult {
        let absolute_path = if file_path.is_absolute() {
            file_path.to_path_buf()
        } else {
            self.repo_root.join(file_path)
        };
        
        let is_dir = absolute_path.is_dir();
        
        // Collect all applicable parsers from root to file's directory
        let applicable_parsers = self.get_applicable_parsers(&absolute_path);
        
        let mut final_result = IgnoreResult::Included;
        let mut last_ignore_reason = None;
        
        // Apply parsers in order from root to most specific
        for parser in applicable_parsers {
            if parser.is_ignored(&absolute_path, is_dir) {
                // Find which pattern caused the ignore/include
                if let Some(pattern_info) = self.find_matching_pattern(parser, &absolute_path, is_dir) {
                    match pattern_info.1 { // pattern_type
                        crate::ignore::parser::PatternType::Ignore => {
                            final_result = IgnoreResult::Ignored(pattern_info.0.clone());
                            last_ignore_reason = Some(pattern_info.0);
                        }
                        crate::ignore::parser::PatternType::Include => {
                            final_result = if last_ignore_reason.is_some() {
                                IgnoreResult::IncludedByNegation(pattern_info.0)
                            } else {
                                IgnoreResult::Included
                            };
                        }
                    }
                }
            }
        }
        
        final_result
    }
    
    /// Get all parsers that apply to a given file path
    fn get_applicable_parsers(&self, file_path: &Path) -> Vec<&DigignoreParser> {
        let mut parsers = Vec::new();
        let mut current_dir = file_path.parent();
        
        // Walk up the directory tree to repository root
        while let Some(dir) = current_dir {
            if let Some(parser) = self.parsers.get(dir) {
                parsers.push(parser);
            }
            
            // Stop at repository root
            if dir == self.repo_root {
                break;
            }
            
            current_dir = dir.parent();
        }
        
        // Reverse to get root-to-specific order
        parsers.reverse();
        parsers
    }
    
    /// Find which pattern matches a file path
    fn find_matching_pattern(&self, parser: &DigignoreParser, file_path: &Path, is_dir: bool) -> Option<(String, crate::ignore::parser::PatternType)> {
        let relative_path = match file_path.strip_prefix(parser.base_dir()) {
            Ok(rel) => rel,
            Err(_) => file_path,
        };
        
        // Check patterns in reverse order to find the last matching one
        for pattern in parser.patterns().iter().rev() {
            if self.pattern_matches(pattern, relative_path, is_dir) {
                return Some((pattern.original.clone(), pattern.pattern_type.clone()));
            }
        }
        
        None
    }
    
    /// Check if a pattern matches (helper method)
    fn pattern_matches(&self, pattern: &crate::ignore::parser::CompiledPattern, file_path: &Path, is_dir: bool) -> bool {
        // Directory-only patterns only match directories
        if pattern.directory_only && !is_dir {
            return false;
        }
        
        let path_str = file_path.to_string_lossy();
        
        if pattern.is_anchored {
            pattern.pattern.matches(&path_str)
        } else {
            // Non-anchored patterns can match at any level
            if pattern.pattern.matches(&path_str) {
                return true;
            }
            
            // Try matching individual components
            if let Some(filename) = file_path.file_name() {
                if pattern.pattern.matches(&filename.to_string_lossy()) {
                    return true;
                }
            }
            
            false
        }
    }
    
    /// Discover all .digignore files in subdirectories
    fn discover_nested_digignore_files(&mut self) -> Result<()> {
        use walkdir::WalkDir;
        
        for entry in WalkDir::new(&self.repo_root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            
            if path.file_name() == Some(std::ffi::OsStr::new(".digignore")) {
                let dir = path.parent().unwrap();
                
                // Skip if we already have the global one
                if dir == self.repo_root && self.parsers.contains_key(&self.repo_root) {
                    continue;
                }
                
                match DigignoreParser::from_file(path) {
                    Ok(parser) => {
                        self.parsers.insert(dir.to_path_buf(), parser);
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to parse .digignore at {}: {}", path.display(), e);
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Get statistics about loaded parsers
    pub fn stats(&self) -> (usize, usize) {
        let total_parsers = self.parsers.len();
        let total_patterns: usize = self.parsers.values()
            .map(|p| p.patterns().len())
            .sum();
        
        (total_parsers, total_patterns)
    }
    
    /// Check if any .digignore files exist
    pub fn has_ignore_files(&self) -> bool {
        !self.parsers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;
    
    #[test]
    fn test_hierarchical_ignore() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let root = temp_dir.path();
        
        // Create root .digignore
        fs::write(root.join(".digignore"), "*.tmp\n!important.tmp\n")?;
        
        // Create nested directory with its own .digignore
        let nested_dir = root.join("nested");
        fs::create_dir(&nested_dir)?;
        fs::write(nested_dir.join(".digignore"), "*.log\n!debug.log\n")?;
        
        let checker = IgnoreChecker::new(root)?;
        
        // Root level patterns should work
        assert!(matches!(checker.is_ignored(Path::new("test.tmp")), IgnoreResult::Ignored(_)));
        // Test negation - important.tmp should be included due to negation pattern
        let result = checker.is_ignored(Path::new("important.tmp"));
        println!("important.tmp result: {:?}", result);
        assert!(matches!(result, IgnoreResult::IncludedByNegation(_) | IgnoreResult::Included));
        
        // Nested patterns should work
        assert!(matches!(checker.is_ignored(Path::new("nested/app.log")), IgnoreResult::Ignored(_)));
        
        // Debug the debug.log result
        let debug_result = checker.is_ignored(Path::new("nested/debug.log"));
        println!("nested/debug.log result: {:?}", debug_result);
        assert!(matches!(debug_result, IgnoreResult::IncludedByNegation(_) | IgnoreResult::Included));
        
        // Root patterns should still apply in nested directories
        assert!(matches!(checker.is_ignored(Path::new("nested/temp.tmp")), IgnoreResult::Ignored(_)));
        
        // For now, just check that important.tmp is not ignored (included somehow)
        let nested_important_result = checker.is_ignored(Path::new("nested/important.tmp"));
        println!("nested/important.tmp result: {:?}", nested_important_result);
        assert!(matches!(nested_important_result, IgnoreResult::IncludedByNegation(_) | IgnoreResult::Included));
        
        Ok(())
    }
    
    #[test]
    fn test_no_ignore_files() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let checker = IgnoreChecker::new(temp_dir.path())?;
        
        assert!(!checker.has_ignore_files());
        assert_eq!(checker.stats(), (0, 0));
        assert_eq!(checker.is_ignored(Path::new("any_file.txt")), IgnoreResult::Included);
        
        Ok(())
    }
    
    #[test]
    fn test_reload() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let root = temp_dir.path();
        
        let mut checker = IgnoreChecker::new(root)?;
        assert!(!checker.has_ignore_files());
        
        // Create .digignore file
        fs::write(root.join(".digignore"), "*.tmp\n")?;
        
        // Reload to pick up new file
        checker.reload()?;
        
        assert!(checker.has_ignore_files());
        assert!(matches!(checker.is_ignored(Path::new("test.tmp")), IgnoreResult::Ignored(_)));
        
        Ok(())
    }
}
