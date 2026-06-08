fn main() {
    #[cfg(feature = "risc0")]
    risc0_build::embed_methods();
}
