//! Collateral management for datastore coins

use crate::core::error::{DigstoreError, Result};
use crate::datastore_coin::types::CollateralConfig;
use serde::{Deserialize, Serialize};

/// Collateral requirement calculation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollateralRequirement {
    /// Base collateral amount required
    pub base_amount: u64,
    
    /// Additional collateral for large datastores
    pub size_multiplier: f64,
    
    /// Total collateral required
    pub total_amount: u64,
    
    /// Breakdown of the calculation
    pub breakdown: CollateralBreakdown,
}

/// Breakdown of collateral calculation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollateralBreakdown {
    /// Size of the datastore in bytes
    pub size_bytes: u64,
    
    /// Size of the datastore in GB
    pub size_gb: f64,
    
    /// Rate per GB in DIG tokens
    pub rate_per_gb_dig: f64,
    
    /// Base calculation in DIG tokens
    pub base_calculation_dig: f64,
    
    /// Whether large datastore multiplier was applied
    pub is_large_datastore: bool,
    
    /// Applied multiplier
    pub applied_multiplier: f64,
}

/// Manages collateral calculations and requirements
pub struct CollateralManager {
    config: CollateralConfig,
}

impl CollateralManager {
    /// Create a new collateral manager with default config
    pub fn new() -> Self {
        Self {
            config: CollateralConfig::default(),
        }
    }
    
    /// Create a new collateral manager with custom config
    pub fn with_config(config: CollateralConfig) -> Self {
        Self { config }
    }
    
    /// Calculate collateral requirement for a datastore
    pub fn calculate_requirement(&self, size_bytes: u64) -> Result<CollateralRequirement> {
        if size_bytes == 0 {
            return Err(DigstoreError::ValidationError {
                field: "size_bytes".to_string(),
                reason: "Datastore size cannot be zero".to_string(),
            });
        }
        
        // Convert bytes to GB
        let size_gb = size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        
        // Base calculation in DIG tokens
        let base_calculation_dig = size_gb * self.config.min_collateral_per_gb_dig;
        
        // Check if large datastore multiplier applies
        let (is_large, multiplier) = if size_bytes > self.config.max_size_standard {
            (true, self.config.large_datastore_multiplier)
        } else {
            (false, 1.0)
        };
        
        // Calculate total with multiplier
        let total_dig = base_calculation_dig * multiplier;
        
        // Convert to u64 (representing DIG tokens with appropriate precision)
        // Using 8 decimal places for DIG tokens
        let base_amount = (base_calculation_dig * 100_000_000.0) as u64;
        let total_amount = (total_dig * 100_000_000.0) as u64;
        
        let breakdown = CollateralBreakdown {
            size_bytes,
            size_gb,
            rate_per_gb_dig: self.config.min_collateral_per_gb_dig,
            base_calculation_dig,
            is_large_datastore,
            applied_multiplier: multiplier,
        };
        
        Ok(CollateralRequirement {
            base_amount,
            size_multiplier: multiplier,
            total_amount,
            breakdown,
        })
    }
    
    /// Verify that provided collateral meets requirements
    pub fn verify_collateral(
        &self,
        size_bytes: u64,
        provided_amount: u64,
    ) -> Result<bool> {
        let requirement = self.calculate_requirement(size_bytes)?;
        Ok(provided_amount >= requirement.total_amount)
    }
    
    /// Get the current configuration
    pub fn get_config(&self) -> &CollateralConfig {
        &self.config
    }
    
    /// Update the configuration
    pub fn update_config(&mut self, config: CollateralConfig) {
        self.config = config;
    }
    
    /// Calculate refundable collateral after grace period
    pub fn calculate_refund(
        &self,
        original_amount: u64,
        elapsed_seconds: u64,
    ) -> u64 {
        if elapsed_seconds < self.config.grace_period_seconds {
            // Still in grace period, no refund
            0
        } else {
            // Full refund after grace period
            original_amount
        }
    }
}

impl Default for CollateralManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_collateral_calculation() {
        let manager = CollateralManager::new();
        
        // Test standard size
        let req = manager.calculate_requirement(1024 * 1024).unwrap(); // 1 MB
        assert_eq!(req.base_amount, 1024 * 1024); // 1 mojo per byte
        assert_eq!(req.total_amount, 1024 * 1024);
        assert!(!req.breakdown.is_large_datastore);
        
        // Test large datastore
        let large_size = 2u64 * 1024 * 1024 * 1024; // 2 GB
        let req = manager.calculate_requirement(large_size).unwrap();
        assert!(req.breakdown.is_large_datastore);
        assert_eq!(req.size_multiplier, 1.5);
        assert_eq!(req.total_amount, (large_size as f64 * 1.5) as u64);
    }
    
    #[test]
    fn test_collateral_verification() {
        let manager = CollateralManager::new();
        let size = 1024 * 1024; // 1 MB
        
        // Exact amount
        assert!(manager.verify_collateral(size, 1024 * 1024).unwrap());
        
        // More than required
        assert!(manager.verify_collateral(size, 2 * 1024 * 1024).unwrap());
        
        // Less than required
        assert!(!manager.verify_collateral(size, 1024 * 1024 - 1).unwrap());
    }
    
    #[test]
    fn test_refund_calculation() {
        let manager = CollateralManager::new();
        let amount = 1000000;
        
        // Within grace period
        assert_eq!(manager.calculate_refund(amount, 0), 0);
        assert_eq!(manager.calculate_refund(amount, 86400), 0); // 1 day
        
        // After grace period (default 30 days)
        assert_eq!(manager.calculate_refund(amount, 86400 * 31), amount);
    }
}