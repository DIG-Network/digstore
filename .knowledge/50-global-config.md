# Global Configuration Specification

## Overview

Digstore Min supports global configuration settings that apply to all repositories. These settings are stored in the user's home directory and provide defaults for various operations.

## Configuration File Location

- **Primary**: `~/.dig/config.toml`
- **Format**: TOML
- **Created**: Automatically on first use with defaults

## Configuration Fields

### User Identity

```toml
[user]
name = "not-disclosed"        # Default value
email = "not-disclosed"       # Default value
```

- **name**: Author name used in commits
  - Default: `"not-disclosed"`
  - Optional: Users can set a real name if desired
  - Used in: Layer metadata, commit information

- **email**: Author email used in commits
  - Default: `"not-disclosed"`
  - Optional: Users can set a real email if desired
  - Used in: Layer metadata, commit information

### Core Settings

```toml
[core]
chunk_size = 65536           # 64KB default
compression = "zstd"         # Compression algorithm
compression_level = 3        # Compression level (1-22 for zstd)
delta_chain_limit = 10       # Maximum delta chain depth
```

### Performance Settings

```toml
[performance]
parallel_threads = 0         # 0 = auto-detect CPU cores
index_cache_size = 104857600 # 100MB default
memory_limit = 0             # 0 = no limit
```

## Default Configuration

When no configuration file exists, Digstore Min uses these defaults:

```toml
# ~/.dig/config.toml

[user]
name = "not-disclosed"
email = "not-disclosed"

[core]
chunk_size = 65536
compression = "zstd"
compression_level = 3
delta_chain_limit = 10

[performance]
parallel_threads = 0
index_cache_size = 104857600
memory_limit = 0
```

## Setting Configuration

### Via CLI

```bash
# Set user name (optional)
digstore config user.name "John Doe"

# Set user email (optional)
digstore config user.email "john@example.com"

# View current configuration
digstore config --list

# Get specific value
digstore config user.name
```

### Via Direct Edit

Users can edit `~/.dig/config.toml` directly:

```toml
[user]
name = "Jane Developer"
email = "jane@example.com"
```

## Privacy by Default

The design philosophy of Digstore Min prioritizes privacy:

1. **No mandatory identity**: Author information is never required
2. **Safe defaults**: "not-disclosed" prevents accidental information leakage
3. **Explicit opt-in**: Users must actively choose to share identity
4. **No telemetry**: No usage data or statistics are collected

## Command-Line Override

Individual commands can override global settings:

```bash
# Override author for single commit
digstore commit -m "Message" --author "Special Author"

# This does NOT change global config
```

## Configuration Precedence

1. Command-line flags (highest priority)
2. Environment variables (DIGSTORE_*)
3. Repository-specific config (future)
4. Global config file
5. Built-in defaults (lowest priority)

## Implementation Notes

### Config Loading

```rust
impl GlobalConfig {
    pub fn load() -> Result<Self> {
        let config_path = dirs::home_dir()
            .ok_or(DigstoreError::NoHomeDirectory)?
            .join(".dig")
            .join("config.toml");
            
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            toml::from_str(&content)
                .map_err(|e| DigstoreError::ConfigParse(e))
        } else {
            // Return defaults with "not-disclosed"
            Ok(Self::default())
        }
    }
    
    pub fn save(&self) -> Result<()> {
        let config_dir = dirs::home_dir()
            .ok_or(DigstoreError::NoHomeDirectory)?
            .join(".dig");
            
        std::fs::create_dir_all(&config_dir)?;
        
        let config_path = config_dir.join("config.toml");
        let content = toml::to_string_pretty(self)?;
        std::fs::write(config_path, content)?;
        
        Ok(())
    }
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            user: UserConfig {
                name: "not-disclosed".to_string(),
                email: "not-disclosed".to_string(),
            },
            core: CoreConfig::default(),
            performance: PerformanceConfig::default(),
        }
    }
}
```

## Security Considerations

1. **File permissions**: Config file should be readable only by owner (0600)
2. **No sensitive data**: Never store passwords or keys in config
3. **Path validation**: Ensure config paths don't escape ~/.dig
4. **Default safety**: Defaults should be secure and private

## Future Extensions

Potential additions (not in initial version):

- Repository templates
- Custom aliases
- Plugin configuration
- Remote repository settings
- GPG signing configuration
