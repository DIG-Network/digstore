//! Data encryption using URNs as keys

use crate::core::error::{DigstoreError, Result};
use crate::crypto::derive_key_from_urn;
use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Nonce, Key
};

/// Encrypt data using a URN as the key source
pub fn encrypt_data(data: &[u8], urn: &str) -> Result<Vec<u8>> {
    // Derive encryption key from URN
    let key_bytes = derive_key_from_urn(urn);
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    
    // Create cipher
    let cipher = Aes256Gcm::new(key);
    
    // Generate random nonce
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    
    // Encrypt
    let ciphertext = cipher
        .encrypt(&nonce, data)
        .map_err(|e| DigstoreError::encryption_error(format!("Encryption failed: {}", e)))?;
    
    // Prepend nonce to ciphertext
    let mut result = nonce.to_vec();
    result.extend_from_slice(&ciphertext);
    
    Ok(result)
}

/// Decrypt data using a URN as the key source
pub fn decrypt_data(encrypted_data: &[u8], urn: &str) -> Result<Vec<u8>> {
    // Check minimum size (nonce + at least some data)
    if encrypted_data.len() < 12 {
        return Err(DigstoreError::decryption_error("Invalid encrypted data: too short"));
    }
    
    // Extract nonce and ciphertext
    let (nonce_bytes, ciphertext) = encrypted_data.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    
    // Derive encryption key from URN
    let key_bytes = derive_key_from_urn(urn);
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    
    // Create cipher
    let cipher = Aes256Gcm::new(key);
    
    // Decrypt
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| DigstoreError::decryption_error(format!("Decryption failed: {}", e)))?;
    
    Ok(plaintext)
}

/// Encrypt data using a custom encryption key (hex string)
pub fn encrypt_data_with_key(data: &[u8], custom_key_hex: &str) -> Result<Vec<u8>> {
    // Parse hex key
    let key_bytes = hex::decode(custom_key_hex)
        .map_err(|_| DigstoreError::encryption_error("Invalid hex encryption key"))?;
    
    if key_bytes.len() != 32 {
        return Err(DigstoreError::encryption_error("Encryption key must be 32 bytes (64 hex characters)"));
    }
    
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    
    // Create cipher
    let cipher = Aes256Gcm::new(key);
    
    // Generate random nonce
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    
    // Encrypt
    let ciphertext = cipher
        .encrypt(&nonce, data)
        .map_err(|e| DigstoreError::encryption_error(format!("Encryption failed: {}", e)))?;
    
    // Prepend nonce to ciphertext
    let mut result = nonce.to_vec();
    result.extend_from_slice(&ciphertext);
    
    Ok(result)
}

/// Decrypt data using a custom decryption key (hex string)
pub fn decrypt_data_with_key(encrypted_data: &[u8], custom_key_hex: &str) -> Result<Vec<u8>> {
    // Check minimum size (nonce + at least some data)
    if encrypted_data.len() < 12 {
        return Err(DigstoreError::decryption_error("Invalid encrypted data: too short"));
    }
    
    // Extract nonce and ciphertext
    let (nonce_bytes, ciphertext) = encrypted_data.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    
    // Parse hex key
    let key_bytes = hex::decode(custom_key_hex)
        .map_err(|_| DigstoreError::decryption_error("Invalid hex decryption key"))?;
    
    if key_bytes.len() != 32 {
        return Err(DigstoreError::decryption_error("Decryption key must be 32 bytes (64 hex characters)"));
    }
    
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    
    // Create cipher
    let cipher = Aes256Gcm::new(key);
    
    // Decrypt
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| DigstoreError::decryption_error(format!("Decryption failed: {}", e)))?;
    
    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_encrypt_decrypt() {
        let data = b"Hello, world!";
        let urn = "urn:dig:chia:abc123/file.txt";
        
        // Encrypt
        let encrypted = encrypt_data(data, urn).unwrap();
        
        // Should be different from original
        assert_ne!(encrypted, data);
        
        // Should be longer (nonce + ciphertext + auth tag)
        assert!(encrypted.len() > data.len());
        
        // Decrypt
        let decrypted = decrypt_data(&encrypted, urn).unwrap();
        assert_eq!(decrypted, data);
    }
    
    #[test]
    fn test_wrong_urn_fails() {
        let data = b"Hello, world!";
        let urn1 = "urn:dig:chia:abc123/file.txt";
        let urn2 = "urn:dig:chia:abc123/file2.txt";
        
        // Encrypt with urn1
        let encrypted = encrypt_data(data, urn1).unwrap();
        
        // Try to decrypt with urn2 - should fail
        let result = decrypt_data(&encrypted, urn2);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_same_urn_different_encryptions() {
        let data = b"Hello, world!";
        let urn = "urn:dig:chia:abc123/file.txt";
        
        // Encrypt twice
        let encrypted1 = encrypt_data(data, urn).unwrap();
        let encrypted2 = encrypt_data(data, urn).unwrap();
        
        // Should be different (due to random nonce)
        assert_ne!(encrypted1, encrypted2);
        
        // But both should decrypt correctly
        let decrypted1 = decrypt_data(&encrypted1, urn).unwrap();
        let decrypted2 = decrypt_data(&encrypted2, urn).unwrap();
        assert_eq!(decrypted1, data);
        assert_eq!(decrypted2, data);
    }
}
