# Icon and File Association Setup

## Icon Configuration

Digstore uses `DIG-Token-450.ico` as its application icon. This icon:
- Features a purple gradient circular design with a white refresh/cycle symbol
- Is embedded in the Windows executable during build
- Is used for the application in Windows Explorer and Start Menu
- Is associated with .dig files

## File Association

The Windows installer configures .dig file association automatically:

### What it does:
1. Associates `.dig` files with Digstore
2. Sets the DIG-Token-450.ico as the icon for .dig files
3. Configures double-click to open .dig files with Digstore
4. Adds "Open with Digstore" to right-click context menu

### Registry Changes:
- Creates ProgId: `Digstore.DigFile`
- Sets file type description: "DIG Archive File"
- Associates .dig extension with the ProgId
- Sets up the open verb with command line arguments

### Command Line:
When a .dig file is opened, Digstore is called with:
```
digstore.exe "%1"
```
Where %1 is the path to the .dig file.

## Icon Requirements

### Windows (.ico)
- Current icon: `DIG-Token-450.ico`
- Should contain multiple sizes for best appearance
- Common sizes: 16x16, 32x32, 48x48, 256x256

### Build Process
The icon is embedded during build using:
1. `build.rs` script with `winres` crate
2. Sets ProductName, CompanyName, and other metadata
3. Embeds icon in the .exe resource section

## Testing File Association

After installation:
1. Create a test .dig file
2. The file should show the Digstore icon
3. Double-clicking should launch Digstore with the file path
4. Right-click → "Open with" should show Digstore

## Troubleshooting

### Icon not showing on .exe:
- Ensure `DIG-Token-450.ico` exists in project root
- Check that `winres` is in `[build-dependencies]`
- Rebuild with `cargo clean && cargo build --release`

### File association not working:
- Run installer as Administrator
- Check Windows Default Apps settings
- May need to log out/in for changes to take effect
- Verify registry entries under `HKEY_CLASSES_ROOT\.dig`

### Icon not showing on .dig files:
- Windows may cache icons - try refreshing icon cache
- Restart Windows Explorer
- Check that installer completed successfully
