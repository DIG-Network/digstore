//! Utility functions for datastore coin operations

use std::fmt;

/// DIG token precision (8 decimal places)
pub const DIG_PRECISION: u64 = 100_000_000;

/// Convert DIG token amount (as u64 with 8 decimal precision) to f64
pub fn dig_to_float(amount: u64) -> f64 {
    amount as f64 / DIG_PRECISION as f64
}

/// Convert float DIG amount to u64 with 8 decimal precision
pub fn float_to_dig(amount: f64) -> u64 {
    (amount * DIG_PRECISION as f64) as u64
}

/// Format DIG token amount for display
pub fn format_dig(amount: u64) -> String {
    format!("{:.8} DIG", dig_to_float(amount))
}

/// Format DIG token amount with custom precision
pub fn format_dig_precision(amount: u64, precision: usize) -> String {
    format!("{:.prec$} DIG", dig_to_float(amount), prec = precision)
}

/// Represents a DIG token amount
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DigAmount(u64);

impl DigAmount {
    /// Create from raw u64 value (with 8 decimal precision)
    pub fn from_raw(value: u64) -> Self {
        Self(value)
    }
    
    /// Create from DIG token float value
    pub fn from_dig(value: f64) -> Self {
        Self(float_to_dig(value))
    }
    
    /// Get raw u64 value
    pub fn raw(&self) -> u64 {
        self.0
    }
    
    /// Get as float DIG value
    pub fn as_dig(&self) -> f64 {
        dig_to_float(self.0)
    }
}

impl fmt::Display for DigAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", format_dig(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_dig_conversions() {
        // Test conversion to/from float
        assert_eq!(dig_to_float(100_000_000), 1.0);
        assert_eq!(dig_to_float(50_000_000), 0.5);
        assert_eq!(dig_to_float(12_345_678), 0.12345678);
        
        assert_eq!(float_to_dig(1.0), 100_000_000);
        assert_eq!(float_to_dig(0.5), 50_000_000);
        assert_eq!(float_to_dig(0.12345678), 12_345_678);
    }
    
    #[test]
    fn test_dig_formatting() {
        assert_eq!(format_dig(100_000_000), "1.00000000 DIG");
        assert_eq!(format_dig(50_000_000), "0.50000000 DIG");
        assert_eq!(format_dig(12_345_678), "0.12345678 DIG");
        
        assert_eq!(format_dig_precision(100_000_000, 2), "1.00 DIG");
        assert_eq!(format_dig_precision(50_000_000, 4), "0.5000 DIG");
    }
    
    #[test]
    fn test_dig_amount() {
        let amount1 = DigAmount::from_dig(1.5);
        assert_eq!(amount1.raw(), 150_000_000);
        assert_eq!(amount1.as_dig(), 1.5);
        assert_eq!(amount1.to_string(), "1.50000000 DIG");
        
        let amount2 = DigAmount::from_raw(75_000_000);
        assert_eq!(amount2.as_dig(), 0.75);
    }
}