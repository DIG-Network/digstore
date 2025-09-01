//! Configuration command implementation

use anyhow::Result;
use crate::config::{GlobalConfig, ConfigKey, ConfigValue};
use colored::Colorize;
use clap::Args;

#[derive(Args)]
pub struct ConfigArgs {
    /// Configuration key to get/set
    pub key: Option<String>,
    
    /// Configuration value to set
    pub value: Option<String>,
    
    /// List all configuration values
    #[arg(short, long)]
    pub list: bool,
    
    /// Unset a configuration value
    #[arg(long)]
    pub unset: bool,
    
    /// Show global configuration file location
    #[arg(long)]
    pub show_origin: bool,
    
    /// Edit configuration file in editor
    #[arg(short, long)]
    pub edit: bool,
    
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

/// Execute the config command
pub fn execute(
    key: Option<String>,
    value: Option<String>,
    list: bool,
    unset: bool,
    show_origin: bool,
    edit: bool,
    json: bool,
) -> Result<()> {
    let args = ConfigArgs { key, value, list, unset, show_origin, edit, json };

    let mut config = GlobalConfig::load()?;

    if args.show_origin {
        let config_path = GlobalConfig::get_config_path()?;
        if json {
            println!("{}", serde_json::json!({
                "config_file": config_path.display().to_string(),
                "exists": config_path.exists()
            }));
        } else {
            println!("{}: {}", "Configuration file".bold(), config_path.display().to_string().cyan());
            if config_path.exists() {
                println!("{}: {}", "Status".bold(), "exists".green());
            } else {
                println!("{}: {}", "Status".bold(), "not created yet".yellow());
            }
        }
        return Ok(());
    }

    if args.edit {
        return edit_config_file();
    }

    if args.list {
        return list_configuration(&config, args.json);
    }

    if let Some(key_str) = &args.key {
        let config_key = ConfigKey::from_str(key_str)
            .ok_or_else(|| anyhow::anyhow!("Invalid configuration key: {}", key_str))?;

        if args.unset {
            // Unset the value
            config.unset(&config_key);
            config.save()?;
            
            if json {
                println!("{}", serde_json::json!({
                    "action": "unset",
                    "key": key_str,
                    "status": "success"
                }));
            } else {
                println!("{} {}", "✓".green(), format!("Unset {}", key_str).bold());
            }
        } else if let Some(value_str) = &args.value {
            // Set the value
            let config_value = parse_config_value(value_str);
            config.set(config_key, config_value)?;
            config.save()?;
            
            if json {
                println!("{}", serde_json::json!({
                    "action": "set",
                    "key": key_str,
                    "value": value_str,
                    "status": "success"
                }));
            } else {
                println!("{} {} = {}", "✓".green(), key_str.bold(), value_str.cyan());
            }
        } else {
            // Get the value
            let config_key = ConfigKey::from_str(key_str).unwrap();
            if let Some(value) = config.get(&config_key) {
                let value_str = format_config_value(&value);
                if json {
                    println!("{}", serde_json::json!({
                        "key": key_str,
                        "value": value_str
                    }));
                } else {
                    println!("{}", value_str);
                }
            } else {
                if json {
                    println!("{}", serde_json::json!({
                        "key": key_str,
                        "value": null,
                        "error": "not set"
                    }));
                } else {
                    eprintln!("{}", format!("Configuration key '{}' is not set", key_str).yellow());
                    return Err(anyhow::anyhow!("Configuration key not found"));
                }
            }
        }
    } else {
        // No key specified - show usage
        if json {
            println!("{}", serde_json::json!({
                "error": "No configuration key specified",
                "usage": "digstore config <key> [value] or --list"
            }));
        } else {
            println!("{}", "Configuration Management".green().bold());
            println!("{}", "═".repeat(40));
            println!();
            println!("{}", "Usage:".bold());
            println!("  {} Get value", "digstore config <key>".cyan());
            println!("  {} Set value", "digstore config <key> <value>".cyan());
            println!("  {} List all", "digstore config --list".cyan());
            println!("  {} Unset value", "digstore config --unset <key>".cyan());
            println!("  {} Edit in editor", "digstore config --edit".cyan());
            println!();
            println!("{}", "Common keys:".bold());
            println!("  {} Your name for commits", "user.name".green());
            println!("  {} Your email for commits", "user.email".green());
            println!("  {} Default editor", "core.editor".green());
            println!("  {} Default chunk size", "core.chunk_size".green());
            println!("  {} Default compression", "core.compression".green());
        }
    }

    Ok(())
}

/// List all configuration values
fn list_configuration(config: &GlobalConfig, json: bool) -> Result<()> {
    let entries = config.list();

    if json {
        let config_map: std::collections::HashMap<String, String> = entries.into_iter().collect();
        println!("{}", serde_json::to_string_pretty(&config_map)?);
    } else {
        if entries.is_empty() {
            println!("{}", "No configuration values set".yellow());
            println!();
            println!("{}", "To set configuration:".bold());
            println!("  {}", "digstore config user.name \"Your Name\"".cyan());
            println!("  {}", "digstore config user.email \"your@email.com\"".cyan());
        } else {
            println!("{}", "Global Configuration".green().bold());
            println!("{}", "═".repeat(40));
            println!();
            
            for (key, value) in entries {
                println!("{} = {}", key.bold(), value.cyan());
            }
        }
    }

    Ok(())
}

/// Edit configuration file in editor
fn edit_config_file() -> Result<()> {
    use std::process::Command;
    
    let config_path = GlobalConfig::get_config_path()?;
    
    // Create config file if it doesn't exist
    if !config_path.exists() {
        let config = GlobalConfig::default();
        config.save()?;
    }
    
    // Get editor from environment or config
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });
    
    println!("{} Opening config file in {}...", "•".cyan(), editor.cyan());
    
    // Open editor
    let status = Command::new(&editor)
        .arg(&config_path)
        .status()?;
    
    if !status.success() {
        return Err(anyhow::anyhow!("Editor exited with non-zero status"));
    }
    
    // Validate the edited configuration
    match GlobalConfig::load() {
        Ok(_) => {
            println!("{} Configuration updated successfully", "✓".green());
        }
        Err(e) => {
            eprintln!("{} Configuration file has errors: {}", "✗".red(), e);
            eprintln!("Please fix the configuration file manually or run 'digstore config --list' to reset");
            return Err(anyhow::anyhow!("Invalid configuration file"));
        }
    }
    
    Ok(())
}

/// Parse a string value into appropriate ConfigValue
fn parse_config_value(value_str: &str) -> ConfigValue {
    // Try to parse as number
    if let Ok(num) = value_str.parse::<i64>() {
        return ConfigValue::Number(num);
    }
    
    // Try to parse as boolean
    match value_str.to_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => return ConfigValue::Boolean(true),
        "false" | "no" | "off" | "0" => return ConfigValue::Boolean(false),
        _ => {}
    }
    
    // Default to string
    ConfigValue::String(value_str.to_string())
}

/// Format a ConfigValue for display
fn format_config_value(value: &ConfigValue) -> String {
    match value {
        ConfigValue::String(s) => s.clone(),
        ConfigValue::Number(n) => n.to_string(),
        ConfigValue::Boolean(b) => b.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config_value() {
        assert!(matches!(parse_config_value("123"), ConfigValue::Number(123)));
        assert!(matches!(parse_config_value("true"), ConfigValue::Boolean(true)));
        assert!(matches!(parse_config_value("false"), ConfigValue::Boolean(false)));
        assert!(matches!(parse_config_value("hello"), ConfigValue::String(_)));
    }

    #[test]
    fn test_format_config_value() {
        assert_eq!(format_config_value(&ConfigValue::String("test".to_string())), "test");
        assert_eq!(format_config_value(&ConfigValue::Number(42)), "42");
        assert_eq!(format_config_value(&ConfigValue::Boolean(true)), "true");
    }
}
