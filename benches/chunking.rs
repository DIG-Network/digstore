use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId, Throughput};
use digstore_min::storage::chunk::ChunkingEngine;


fn chunking_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("chunking");
    
    // Test different data sizes
    let sizes = vec![1024, 8192, 65536, 1048576, 8388608]; // 1KB to 8MB
    
    for size in sizes {
        group.throughput(Throughput::Bytes(size as u64));
        
        // Generate test data
        let data = generate_test_data(size);
        let engine = ChunkingEngine::new();
        
        group.bench_with_input(
            BenchmarkId::new("chunk_data", size),
            &data,
            |b, data| {
                b.iter(|| {
                    engine.chunk_data(black_box(data)).unwrap()
                })
            },
        );
    }
    
    group.finish();
}

fn hashing_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("hashing");
    
    // Test different data sizes for hashing
    let sizes = vec![1024, 8192, 65536, 1048576, 8388608]; // 1KB to 8MB
    
    for size in sizes {
        group.throughput(Throughput::Bytes(size as u64));
        
        let data = generate_test_data(size);
        
        group.bench_with_input(
            BenchmarkId::new("sha256", size),
            &data,
            |b, data| {
                b.iter(|| {
                    digstore_min::core::hash::sha256(black_box(data))
                })
            },
        );
    }
    
    group.finish();
}

fn merkle_tree_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("merkle_tree");
    
    // Test different numbers of hashes
    let hash_counts = vec![1, 10, 100, 1000, 10000];
    
    for count in hash_counts {
        group.throughput(Throughput::Elements(count as u64));
        
        // Generate test hashes
        let hashes: Vec<digstore_min::core::types::Hash> = (0..count)
            .map(|i| {
                let data = format!("test_data_{}", i);
                digstore_min::core::hash::sha256(data.as_bytes())
            })
            .collect();
        
        group.bench_with_input(
            BenchmarkId::new("build_tree", count),
            &hashes,
            |b, hashes| {
                b.iter(|| {
                    digstore_min::proofs::merkle::MerkleTree::from_hashes(black_box(hashes)).unwrap()
                })
            },
        );
        
        // Benchmark proof generation
        if count > 1 {
            let tree = digstore_min::proofs::merkle::MerkleTree::from_hashes(&hashes).unwrap();
            let leaf_index = count / 2; // Middle element
            
            group.bench_with_input(
                BenchmarkId::new("generate_proof", count),
                &(tree, leaf_index),
                |b, (tree, leaf_index)| {
                    b.iter(|| {
                        tree.generate_proof(black_box(*leaf_index)).unwrap()
                    })
                },
            );
        }
    }
    
    group.finish();
}

fn file_operations_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("file_operations");
    
    // Create temporary test files of different sizes
    let sizes = vec![1024, 65536, 1048576]; // 1KB, 64KB, 1MB
    
    for size in sizes {
        group.throughput(Throughput::Bytes(size as u64));
        
        // Create temporary file
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join(format!("test_{}.dat", size));
        let data = generate_test_data(size);
        std::fs::write(&file_path, &data).unwrap();
        
        let engine = ChunkingEngine::new();
        
        group.bench_with_input(
            BenchmarkId::new("chunk_file", size),
            &file_path,
            |b, file_path| {
                b.iter(|| {
                    engine.chunk_file(black_box(file_path)).unwrap()
                })
            },
        );
    }
    
    group.finish();
}

fn generate_test_data(size: usize) -> Vec<u8> {
    // Generate pseudo-random but deterministic data
    let mut data = Vec::with_capacity(size);
    let mut state = 12345u64;
    
    for _ in 0..size {
        // Simple LCG for deterministic "random" data
        state = state.wrapping_mul(1103515245).wrapping_add(12345);
        data.push((state >> 8) as u8);
    }
    
    data
}

criterion_group!(
    benches,
    chunking_benchmark,
    hashing_benchmark,
    merkle_tree_benchmark,
    file_operations_benchmark
);
criterion_main!(benches);
