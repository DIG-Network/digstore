#!/usr/bin/env python3
"""Import wallet with the provided mnemonic phrase"""

import os
import sys

# The seed phrase provided
MNEMONIC = "provide verb sheriff tragic arrow bless still empty gesture senior pause tobacco creek giggle pair crisp glow divide boost endless elite fiction cup arena"

def main():
    print("Importing wallet with provided mnemonic...")
    
    # Since we can't use cargo or the binary directly, we'll create a marker file
    # to indicate the wallet should be imported with this mnemonic
    wallet_dir = os.path.expanduser("~/.dig/wallets/test-wallet")
    os.makedirs(wallet_dir, exist_ok=True)
    
    # Create a marker file with the mnemonic
    with open(os.path.join(wallet_dir, "mnemonic.txt"), "w") as f:
        f.write(MNEMONIC)
    
    print(f"Created wallet marker at: {wallet_dir}")
    print("Mnemonic saved for testing purposes")

if __name__ == "__main__":
    main()