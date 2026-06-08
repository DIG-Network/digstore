use digstore_core::ChunkerConfig;

pub fn mask_for_target(_target_size: usize) -> u64 {
    0
}

pub fn default_config() -> ChunkerConfig {
    ChunkerConfig { min_size: 0, target_size: 0, max_size: 0, mask: 0 }
}
