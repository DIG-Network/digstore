// Build script to embed icon and set Windows executable metadata

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=DIG.ico");
    
    // Only run on Windows
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        println!("cargo:warning=Running Windows build script");
        
        // Create Windows resource file
        let mut res = winres::WindowsResource::new();
        
        // Set the icon - this embeds it in the .exe
        res.set_icon("DIG.ico");
        println!("cargo:warning=Icon set to DIG.ico");
        
        // Set version info
        res.set("ProductName", "Digstore");
        res.set("CompanyName", "DIG Network");
        res.set("LegalCopyright", "Copyright © 2025 DIG Network");
        res.set("FileDescription", "Content-addressable storage system");
        
        // Compile the resource file
        match res.compile() {
            Ok(_) => println!("cargo:warning=Resource compilation successful"),
            Err(e) => println!("cargo:warning=Resource compilation failed: {}", e),
        }
    } else {
        println!("cargo:warning=Not Windows, skipping icon embedding");
    }
}
