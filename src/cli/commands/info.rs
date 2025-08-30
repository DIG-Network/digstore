//! Info command implementation

use anyhow::Result;

/// Execute the info command
pub fn execute(
    json: bool,
    layer: Option<String>,
) -> Result<()> {
    println!("Displaying repository information...");
    println!("  JSON: {}", json);
    println!("  Layer: {:?}", layer);
    
    // TODO: Implement actual info functionality
    println!("✓ Information displayed!");
    
    Ok(())
}
