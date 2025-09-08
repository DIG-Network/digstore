// Build script to embed icon and set Windows executable metadata

fn main() {
    // Only run on Windows
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        println!("cargo:rerun-if-changed=build.rs");
        println!("cargo:rerun-if-changed=DIG.ico");
        println!("cargo:rerun-if-changed=digstore.rc");

        // Create resource file with icon and version info
        let rc_content = r#"
#pragma code_page(65001)
1 ICON "DIG.ico"

1 VERSIONINFO
FILEVERSION     0,1,0,0
PRODUCTVERSION  0,1,0,0
FILEFLAGSMASK   0x3f
FILEFLAGS       0x0
FILEOS          0x40004
FILETYPE        0x1
FILESUBTYPE     0x0
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "CompanyName",      "DIG Network"
            VALUE "FileDescription",  "Content-addressable storage system"
            VALUE "FileVersion",      "0.1.0.0"
            VALUE "InternalName",     "digstore"
            VALUE "LegalCopyright",   "Copyright © 2025 DIG Network"
            VALUE "OriginalFilename", "digstore.exe"
            VALUE "ProductName",      "Digstore"
            VALUE "ProductVersion",   "0.1.0.0"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 0x4B0
    END
END
"#;

        // Write RC file
        std::fs::write("digstore.rc", rc_content).expect("Failed to write RC file");
        println!("cargo:warning=Created digstore.rc file");

        // Use embed-resource to compile and link the resources
        embed_resource::compile("digstore.rc", embed_resource::NONE);
        println!("cargo:warning=Resources compiled and will be linked into executable");
    }
}
