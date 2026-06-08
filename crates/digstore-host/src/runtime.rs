//! `HostRuntime`: wasmtime engine + module + serve flow (§18).

use crate::clock::Clock;
use crate::config::ExecutionLimits;
use crate::error::HostError;
use crate::memory::read_bytes;
use crate::random::HostRng;
use crate::session::SessionTable;
use crate::state::{HostKeys, HostState, ReturnBuffer};
use crate::teehook::{BlsAttestationBackend, SharedBackend};
use digstore_core::abi::{is_error, unpack_ptr_len};
use digstore_core::config::HostImportsConfig;
use digstore_core::types::{Bytes32, Bytes48};
use digstore_crypto::bls::BlsSecretKey;
use digstore_prover::{ChainSource, Prover};
use std::sync::Arc;
use wasmtime::{Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

/// Dependencies injected into a runtime: BLS keys, clock, chain, prover, rng.
pub struct HostDeps {
    pub store_id: Bytes32,
    pub bls_secret: BlsSecretKey,
    pub bls_public: Bytes48,
    pub clock: Arc<dyn Clock>,
    pub chain: Arc<dyn ChainSource>,
    pub prover: Arc<dyn Prover>,
    /// `Some(seed)` => deterministic rng (tests); `None` => OS entropy.
    pub rng_seed: Option<[u8; 32]>,
    pub instance_id: Bytes32,
    /// `None` => default BLS attestation backend built from the BLS keys (§13.6).
    pub attestation: Option<SharedBackend>,
}

/// Combined per-store host state. The wasmtime resource limiter is added in Task 13.
pub struct RuntimeState {
    pub host: HostState,
}

pub struct HostRuntime {
    store: Store<RuntimeState>,
    instance: Instance,
    memory: Memory,
    limits_cfg: ExecutionLimits,
}

impl HostRuntime {
    pub fn new(
        module_bytes: &[u8],
        config: HostImportsConfig,
        limits: ExecutionLimits,
        deps: HostDeps,
    ) -> Result<Self, HostError> {
        let mut wcfg = wasmtime::Config::new();
        wcfg.consume_fuel(true);
        wcfg.epoch_interruption(true);
        let engine = Engine::new(&wcfg).map_err(|e| HostError::Wasmtime(e.to_string()))?;

        Module::validate(&engine, module_bytes)
            .map_err(|e| HostError::Validation(e.to_string()))?;
        let module = Module::new(&engine, module_bytes)
            .map_err(|e| HostError::Wasmtime(e.to_string()))?;

        let rng = match deps.rng_seed {
            Some(s) => HostRng::from_seed(s),
            None => HostRng::from_entropy(),
        };

        // The BLS secret is not `Clone`, so share it (Arc) between HostKeys and
        // the default attestation backend (§13.6 default = BLS backend).
        let shared_secret = Arc::new(deps.bls_secret);
        let attestation: SharedBackend = match deps.attestation {
            Some(b) => b,
            None => Arc::new(BlsAttestationBackend::from_shared(
                shared_secret.clone(),
                deps.bls_public,
            )),
        };

        let host = HostState {
            store_id: deps.store_id,
            config: config.clone(),
            return_buffer: ReturnBuffer::new(&config),
            keys: Arc::new(HostKeys {
                bls_secret: shared_secret,
                bls_public: deps.bls_public,
            }),
            attestation,
            clock: deps.clock,
            sessions: SessionTable::new(),
            chain: deps.chain,
            prover: deps.prover,
            rng,
            instance_id: deps.instance_id,
            http_timeout_secs: limits.timeout.as_secs().max(1),
            last_signature: None,
        };

        let mut store = Store::new(&engine, RuntimeState { host });

        let mut linker: Linker<RuntimeState> = Linker::new(&engine);
        crate::imports::register(&mut linker)?;

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| HostError::Wasmtime(e.to_string()))?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or(HostError::MissingExport("memory"))?;

        if let Ok(init) = instance.get_typed_func::<(), i32>(&mut store, "init") {
            // arm bounds even for init so a malicious init cannot hang setup.
            let _ = store.set_fuel(limits.fuel);
            // Epoch interruption is enabled on the engine; a deadline MUST be set
            // or the first epoch check traps. Task 12 wires the ticker + real
            // deadline; until then use a large deadline so legitimate code runs.
            store.set_epoch_deadline(u64::MAX);
            let _ = init.call(&mut store, ());
        }

        Ok(HostRuntime {
            store,
            instance,
            memory,
            limits_cfg: limits,
        })
    }

    /// Set the per-export-call fuel budget. Epoch deadline is added in Task 12.
    /// NOTE: bounds are armed PER export call (alloc, serve, dealloc each get
    /// their own budget); the serve flow is not a single combined budget (§18.2).
    fn arm_bounds(&mut self) {
        let _ = self.store.set_fuel(self.limits_cfg.fuel);
        // Epoch interruption is enabled; a deadline MUST be set or the first
        // epoch check traps. Task 12 replaces this with a ticker-driven deadline.
        self.store.set_epoch_deadline(u64::MAX);
    }

    fn map_trap(e: wasmtime::Error) -> HostError {
        HostError::Wasmtime(e.to_string())
    }

    /// Unpack a packed ptr/len, check the error sentinel, and read the bytes.
    fn unpack_and_read(&mut self, packed: i64) -> Result<Vec<u8>, HostError> {
        if is_error(packed) {
            let (ptr, _len) = unpack_ptr_len(packed);
            return Err(HostError::from_guest_code(ptr as i32));
        }
        let (ptr, len) = unpack_ptr_len(packed);
        read_bytes(&self.store, &self.memory, ptr, len)
    }

    fn data_export(&mut self, name: &'static str) -> Result<Vec<u8>, HostError> {
        let func: TypedFunc<(), i64> = self
            .instance
            .get_typed_func(&mut self.store, name)
            .map_err(|_| HostError::MissingExport(name))?;
        self.arm_bounds();
        let packed = func.call(&mut self.store, ()).map_err(Self::map_trap)?;
        self.unpack_and_read(packed)
    }

    pub fn get_store_id(&mut self) -> Result<Vec<u8>, HostError> {
        self.data_export("get_store_id")
    }

    pub fn get_current_roothash(&mut self) -> Result<Vec<u8>, HostError> {
        self.data_export("get_current_roothash")
    }
}

impl HostRuntime {
    pub fn call_i64_export(&mut self, name: &str) -> Result<i64, HostError> {
        let f: TypedFunc<(), i64> = self
            .instance
            .get_typed_func(&mut self.store, name)
            .map_err(|_| HostError::MissingExport("i64-export"))?;
        self.arm_bounds();
        f.call(&mut self.store, ()).map_err(Self::map_trap)
    }

    pub fn call_i32_export_1(&mut self, name: &str, arg: i32) -> Result<i32, HostError> {
        let f: TypedFunc<i32, i32> = self
            .instance
            .get_typed_func(&mut self.store, name)
            .map_err(|_| HostError::MissingExport("i32-export-1"))?;
        self.arm_bounds();
        f.call(&mut self.store, arg).map_err(Self::map_trap)
    }

    pub fn read_guest(&mut self, ptr: u32, len: u32) -> Result<Vec<u8>, HostError> {
        crate::memory::read_bytes(&self.store, &self.memory, ptr, len)
    }
}
