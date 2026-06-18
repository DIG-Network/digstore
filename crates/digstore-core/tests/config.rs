use digstore_core::config::{
    ChunkerConfig, CompilerError, GenerationId, GenerationState, HostImportsConfig, SecretSalt,
    StoreConfig, TrustedHostKey, Visibility,
};
use digstore_core::Bytes32;

#[test]
fn chunker_config_defaults() {
    let c = ChunkerConfig::default();
    assert_eq!(c.min_size, 16 * 1024);
    assert_eq!(c.target_size, 64 * 1024);
    assert_eq!(c.max_size, 256 * 1024);
}

#[test]
fn host_imports_config_defaults() {
    let h = HostImportsConfig::default();
    assert_eq!(h.return_buffer_capacity, 64 * 1024);
    assert_eq!(h.max_return_buffer_size, 16 * 1024 * 1024);
    assert_eq!(h.max_random_bytes, 1024);
}

#[test]
fn visibility_variants() {
    let pubv = Visibility::Public;
    let privv = Visibility::Private(SecretSalt([9; 32]));
    assert_ne!(pubv, privv);
    match privv {
        Visibility::Private(SecretSalt(s)) => assert_eq!(s, [9; 32]),
        _ => panic!("expected private"),
    }
}

#[test]
fn store_config_constructs() {
    let cfg = StoreConfig {
        store_id: Bytes32([1; 32]),
        data_dir: "/var/dig".into(),
        max_size: 1024,
        visibility: Visibility::Public,
        label: None,
        description: None,
    };
    assert_eq!(cfg.max_size, 1024);
}

#[test]
fn generation_state_and_id() {
    let id: GenerationId = 7;
    let gs = GenerationState {
        id,
        root: Bytes32([2; 32]),
        timestamp: 100,
    };
    assert_eq!(gs.id, 7);
}

#[test]
fn trusted_host_key_label_form() {
    let key = TrustedHostKey {
        public_key: [3; 48],
        label: "dig-host-key-v1:deadbeef".into(),
    };
    assert!(key.label.starts_with("dig-host-key-v1:"));
}

#[test]
fn compiler_error_no_trusted_keys() {
    let e = CompilerError::NoTrustedKeys;
    assert_eq!(e, CompilerError::NoTrustedKeys);
}
