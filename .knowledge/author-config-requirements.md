# Author Configuration Requirements

## Overview

This document specifies how author name and email are handled in Digstore Min, with a focus on privacy by default.

## Key Requirements

### 1. Optional Author Information
- Author name and email are **completely optional**
- No prompts or requirements to provide real identity
- System works fully without any author configuration

### 2. Default Values
- **Default author name**: `"not-disclosed"`
- **Default author email**: `"not-disclosed"`
- These defaults are used when no configuration is set

### 3. Configuration Levels

#### Global Configuration
Location: `~/.dig/config.toml`

```toml
[user]
name = "not-disclosed"    # Default if not set
email = "not-disclosed"   # Default if not set
```

#### Command-Line Override
```bash
# Override for single commit (does not save to config)
digstore commit -m "message" --author "Custom Name" --email "custom@email.com"

# If not specified, uses global config (or defaults)
digstore commit -m "message"
```

## Usage Examples

### Default Behavior (No Configuration)
```bash
# User has never configured author
digstore init
digstore add file.txt
digstore commit -m "Initial commit"
# Commit will have author: "not-disclosed" <not-disclosed>
```

### Optional Configuration
```bash
# User chooses to set name (completely optional)
digstore config user.name "Alice"
# Email remains "not-disclosed"

# Or set both (completely optional)
digstore config user.name "Bob"
digstore config user.email "bob@example.com"
```

### Viewing Configuration
```bash
# Show current settings
digstore config user.name
> not-disclosed

digstore config user.email  
> not-disclosed

# List all config
digstore config --list
> user.name = not-disclosed
> user.email = not-disclosed
> core.chunk_size = 65536
> ...
```

## Layer Storage

In each layer's metadata:

```json
{
  "timestamp": 1699564800,
  "author": {
    "name": "not-disclosed",
    "email": "not-disclosed"
  },
  "message": "Commit message",
  // ... other fields
}
```

## Privacy Considerations

1. **No information leakage**: Default values prevent accidental exposure
2. **No mandatory fields**: Never require real names or emails
3. **Clear defaults**: "not-disclosed" is obvious and intentional
4. **Explicit opt-in**: Users must actively choose to share identity

## Implementation Details

### Config Loading Logic

```rust
pub struct UserConfig {
    pub name: String,
    pub email: String,
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            name: "not-disclosed".to_string(),
            email: "not-disclosed".to_string(),
        }
    }
}

impl UserConfig {
    pub fn load() -> Self {
        // Try to load from ~/.dig/config.toml
        if let Ok(config) = load_from_file() {
            config
        } else {
            // Return defaults
            Self::default()
        }
    }
}
```

### Commit Creation

```rust
pub fn create_commit(message: &str, author_override: Option<Author>) -> Result<LayerMetadata> {
    // Get author info
    let author = if let Some(override_author) = author_override {
        override_author
    } else {
        // Load from config or use defaults
        let config = UserConfig::load();
        Author {
            name: config.name,
            email: config.email,
        }
    };
    
    // Author will be "not-disclosed" if never configured
    Ok(LayerMetadata {
        author,
        message: message.to_string(),
        timestamp: SystemTime::now(),
        // ... other fields
    })
}
```

## Testing Requirements

1. **Default behavior test**: Verify "not-disclosed" is used without config
2. **Configuration test**: Verify custom values are saved and loaded
3. **Override test**: Verify CLI flags override config
4. **Privacy test**: Ensure no prompts for author information

## Summary

- Author name and email default to "not-disclosed"
- Configuration is completely optional
- Privacy is the default behavior
- Users can choose to provide identity if desired
- No mandatory identity requirements
