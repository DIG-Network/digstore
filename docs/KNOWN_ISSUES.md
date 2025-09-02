# Known Issues

## Empty Files in Existing Repositories (Fixed in commit a417823)

### Issue
Files retrieved using `digstore get` from commits made before commit a417823 will be empty (0 bytes).

### Cause
The binary staging area was storing only chunk metadata (hash, offset, size) but not the actual chunk data. When files were committed, the chunk data field was empty, resulting in empty files when retrieved.

### Fix
The commit process has been updated to read chunk data from the original files at commit time and properly store it in the layer.

### Impact
- **New commits**: Files committed after this fix will store and retrieve correctly
- **Existing commits**: Files from commits before this fix will remain empty
- **Workaround**: Re-add and re-commit files if you need to retrieve them

### Example
```bash
# Files from old commits will be empty
digstore get README.md -o test.txt  # Results in 0-byte file

# To fix, re-add and re-commit the file
digstore add README.md
digstore commit -m "Re-commit with chunk data fix"

# Now retrieval works correctly
digstore get README.md -o test.txt  # File has content
```

## Other Known Issues

### Windows PATH Configuration
- The Windows installer adds digstore to PATH, but you need to restart your terminal or log out/in for changes to take effect
- Some terminals may cache the PATH and require a full restart

### Large File Handling
- Very large files (>1GB) may take significant time to process
- Memory usage scales with file size during chunking

### File Permissions
- File permissions are not fully preserved on Windows
- Unix file modes are stored but may not be restored correctly on Windows systems
