# Test & $digstore get behavior outside of a repository

Write-Host "`nTesting & $digstore get behavior..." -ForegroundColor Blue

# Set up path to digstore binary
$digstore = "C:\Users\micha\workspace\dig_network\digstore_min\target\release\digstore.exe"

# First test: In the digstore repository
Write-Host "`n1. Testing inside digstore repository:" -ForegroundColor Green
Set-Location "C:\Users\micha\workspace\dig_network\digstore_min"

# Create a test file and commit it
Write-Host "Creating and committing test file..."
"This is test content" | Out-File -FilePath "test-file.txt" -Encoding utf8
& $digstore add test-file.txt
& $digstore commit -m "Add test file"

# Now test get command
Write-Host "`nTesting '& $digstore get test-file.txt':"
& $digstore get test-file.txt -o retrieved-test.txt
Write-Host "Content of retrieved file:"
Get-Content retrieved-test.txt

# Clean up
Remove-Item test-file.txt -ErrorAction SilentlyContinue
Remove-Item retrieved-test.txt -ErrorAction SilentlyContinue

# Second test: Outside repository
Write-Host "`n`n2. Testing outside repository:" -ForegroundColor Green
Set-Location "C:\Users\micha\workspace\chia\chia-wallet-sdk"

Write-Host "`nTesting '& $digstore get README.md' (should fail):"
try {
    & $digstore get README.md 2>&1
} catch {
    Write-Host "Error: $_" -ForegroundColor Red
}

Write-Host "`nTesting '& $digstore get .\README.md' (should fail):"
try {
    & $digstore get .\README.md 2>&1
} catch {
    Write-Host "Error: $_" -ForegroundColor Red
}

# Test with output file
Write-Host "`nTesting '& $digstore get README.md -o test.txt' (should fail):"
try {
    & $digstore get README.md -o test.txt 2>&1
    if (Test-Path test.txt) {
        Write-Host "File test.txt was created. Size: $((Get-Item test.txt).Length) bytes" -ForegroundColor Yellow
        Remove-Item test.txt
    }
} catch {
    Write-Host "Error: $_" -ForegroundColor Red
}

# Return to original directory
Set-Location "C:\Users\micha\workspace\dig_network\digstore_min"
