# Debug script to understand get command behavior

Write-Host "`nDebugging digstore get command..." -ForegroundColor Blue

# Set up path to digstore binary
$digstore = "C:\Users\micha\workspace\dig_network\digstore_min\target\release\digstore.exe"

# Go to a directory without a repository
Set-Location "C:\Users\micha\workspace\chia\chia-wallet-sdk"

Write-Host "`nCurrent directory: $(Get-Location)" -ForegroundColor Yellow

# Check for .digstore files in current and parent directories
Write-Host "`nLooking for .digstore files:"
$current = Get-Location
while ($current) {
    $digstoreFile = Join-Path $current ".digstore"
    if (Test-Path $digstoreFile) {
        Write-Host "  Found: $digstoreFile" -ForegroundColor Green
    } else {
        Write-Host "  Not found: $digstoreFile" -ForegroundColor Gray
    }
    
    $parent = Split-Path $current -Parent
    if ($parent -eq $current) { break }
    $current = $parent
}

# Create a test README.md file to see what happens
Write-Host "`nCreating test README.md file..."
"# Test README Content" | Out-File -FilePath "README.md" -Encoding utf8

# Run digstore get with verbose/debug output if available
Write-Host "`nRunning digstore get README.md -o output.txt..."
& $digstore get README.md -o output.txt 2>&1 | Out-String

# Check the output
if (Test-Path output.txt) {
    $size = (Get-Item output.txt).Length
    Write-Host "`nOutput file created. Size: $size bytes" -ForegroundColor Yellow
    
    if ($size -gt 0) {
        Write-Host "Content:" -ForegroundColor Cyan
        Get-Content output.txt
    } else {
        Write-Host "File is empty!" -ForegroundColor Red
    }
    
    Remove-Item output.txt
} else {
    Write-Host "`nNo output file created" -ForegroundColor Red
}

# Clean up
Remove-Item README.md -ErrorAction SilentlyContinue

# Return to original directory
Set-Location "C:\Users\micha\workspace\dig_network\digstore_min"
