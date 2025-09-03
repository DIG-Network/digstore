# .digstore File Specification

## Overview

The `.digstore` file is a TOML-formatted configuration file that links a local project directory to a global Digstore repository. It enables the separation of project files from repository data, allowing multiple projects to share the same versioned data store.

## File Location

- **Path**: `<project-root>/.digstore`
- **Format**: TOML
- **Encoding**: UTF-8

## File Format

### Standard Format

```toml
version = "1.0.0"
store_id = "a3f5c8d9e2b1f4a6c9d8e7f2a5b8c1d4e7f0a3b6c9d2e5f8b1c4d7e0a3b6c9d2"
encrypted = false
created_at = "2023-11-10T12:00:00Z"
last_accessed = "2023-11-10T14:30:00Z"
repository_name = "my-awesome-project"
```



## Field Descriptions

### Required Fields

- **version** (string): Version of the .digstore file format
  - Current version: `"1.0.0"`
  - Used for backward compatibility

- **store_id** (string): The unique identifier of the linked store
  - 64-character hexadecimal string (32 bytes)
  - Links to directory `~/.dig/{store_id}/`

- **encrypted** (boolean): Whether the repository uses encryption
  - Always `false` for digstore_min
  - Reserved for future use

- **created_at** (string): ISO 8601 timestamp when the link was created
  - Format: `YYYY-MM-DDTHH:MM:SSZ`
  - Example: `"2023-11-10T12:00:00Z"`

- **last_accessed** (string): ISO 8601 timestamp of last access
  - Updated automatically when repository is accessed
  - Used for cleanup and usage tracking



### Optional Fields

- **repository_name** (string): Human-readable repository name
  - Helps identify repositories
  - Displayed in status commands
  - Example: `"my-awesome-project"`



## Behavior

### Portable Design

Digstore Min is always portable:

1. **Global directory discovery**:
   - First checks `$HOME/.dig`
   - Falls back to platform-specific locations
   - Creates directory if needed

2. **Working directory**:
   - Uses directory containing `.digstore` file
   - Allows project to be moved freely

3. **Benefits**:
   - Portable between machines
   - Works with different usernames
   - Survives directory moves



## CLI Integration

The `.digstore` file enables simplified CLI commands:

```bash
# Without .digstore (requires full URN):
digstore get urn:dig:chia:abc123/src/main.rs

# With .digstore (in project directory):
digstore get /src/main.rs
digstore cat /README.md
digstore get "/large.bin#bytes=0-1024"
```

## Creation

### Via CLI

```bash
# Creates .digstore in current directory
digstore init

# With custom repository name
digstore init --name "my-project"
```

### Programmatically

```rust
let digstore = DigstoreFile {
    version: "1.0.0".to_string(),
    store_id: store_id.to_string(),
    encrypted: false,
    created_at: Utc::now().to_rfc3339(),
    last_accessed: Utc::now().to_rfc3339(),
    repository_name: Some("my-project".to_string()),
};

digstore.save(Path::new(".digstore"))?;
```

## Security Considerations

1. **No sensitive data**: The file contains no keys or credentials
2. **Safe to commit**: Can be added to version control
3. **Read-only reference**: Only links to store, doesn't contain data



## Error Handling

Common issues and solutions:

1. **Missing .digstore**:
   - Error: "No .digstore file found"
   - Solution: Run `digstore init` in project root

2. **Invalid store_id**:
   - Error: "Store not found: {store_id}"
   - Solution: Verify store exists in `~/.dig/`



## Best Practices

1. **Include repository_name** for clarity
2. **Commit .digstore** to version control
3. **Don't edit store_id** manually
4. **Let CLI manage timestamps**

## Future Extensions

Potential future fields:

- `remote_urls`: For distributed synchronization
- `default_branch`: For multi-branch support
- `hooks`: For pre/post operation scripts
- `metadata`: Custom key-value pairs
