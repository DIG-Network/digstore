// Build script to embed icon and set Windows executable metadata

fn main() {
    // Only run on Windows
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        // Create Windows resource file
        let mut res = winres::WindowsResource::new();
        
        // Set the icon - this embeds it in the .exe
        res.set_icon("DIG.ico");
        
        // Set version info
        res.set("ProductName", "Digstore");
        res.set("CompanyName", "DIG Network");
        res.set("LegalCopyright", "Copyright © 2025 DIG Network");
        res.set("FileDescription", "Content-addressable storage system");
        
        // Compile the resource file
        res.compile().unwrap();
    }
}
