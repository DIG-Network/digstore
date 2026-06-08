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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use wasmtime::{
    Engine, Instance, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc,
};

/// Background thread that increments the engine epoch on a fixed period so the
/// wall-clock timeout (§18.2) is enforced via epoch interruption.
pub struct EpochTicker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl EpochTicker {
    fn start(engine: Engine, period: Duration) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let handle = std::thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                std::thread::sleep(period);
                engine.increment_epoch();
            }
        });
        EpochTicker {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

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

/// Combined per-store host state, including the wasmtime resource limiter that
/// enforces the outer memory ceiling (§18.2).
pub struct RuntimeState {
    pub host: HostState,
    pub limits: StoreLimits,
}

pub struct HostRuntime {
    store: Store<RuntimeState>,
    instance: Instance,
    memory: Memory,
    limits_cfg: ExecutionLimits,
    _ticker: EpochTicker,
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

        // Epoch ticker fires every timeout/2, so a deadline of 2 ticks bounds a
        // single export call to roughly `timeout` of wall-clock time (§18.2).
        let period = (limits.timeout / 2).max(Duration::from_millis(10));
        let ticker = EpochTicker::start(engine.clone(), period);

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

        let store_limits = StoreLimitsBuilder::new()
            .memory_size(limits.memory_bytes_max)
            .build();

        let mut store = Store::new(
            &engine,
            RuntimeState {
                host,
                limits: store_limits,
            },
        );
        store.limiter(|s| &mut s.limits);
        // Epoch-deadline expiration traps (the default, set explicitly for clarity).
        store.epoch_deadline_trap();

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
            // Epoch interruption is enabled; a deadline MUST be set or the first
            // epoch check traps. 2 ticks bounds init to ~`timeout` wall-clock.
            store.set_epoch_deadline(2);
            let _ = init.call(&mut store, ());
        }

        Ok(HostRuntime {
            store,
            instance,
            memory,
            limits_cfg: limits,
            _ticker: ticker,
        })
    }

    /// Set the per-export-call fuel budget. Epoch deadline is added in Task 12.
    /// NOTE: bounds are armed PER export call (alloc, serve, dealloc each get
    /// their own budget); the serve flow is not a single combined budget (§18.2).
    fn arm_bounds(&mut self) {
        let _ = self.store.set_fuel(self.limits_cfg.fuel);
        // Deadline = 2 epoch ticks (the ticker fires every timeout/2).
        // NOTE: bounds are armed per export call, not once per serve sequence (§18.2).
        self.store.set_epoch_deadline(2);
    }

    fn map_trap(e: wasmtime::Error) -> HostError {
        use wasmtime::Trap;
        if let Some(trap) = e.downcast_ref::<Trap>() {
            match trap {
                Trap::Interrupt => return HostError::Timeout,
                Trap::OutOfFuel => return HostError::OutOfFuel,
                _ => {}
            }
        }
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

    pub fn get_public_key(&mut self) -> Result<Vec<u8>, HostError> {
        self.data_export("get_public_key")
    }

    pub fn get_roothash_history(&mut self) -> Result<Vec<u8>, HostError> {
        self.data_export("get_roothash_history")
    }

    pub fn get_metadata(&mut self) -> Result<Vec<u8>, HostError> {
        self.data_export("get_metadata")
    }

    pub fn get_authentication_info(&mut self) -> Result<Vec<u8>, HostError> {
        self.data_export("get_authentication_info")
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

    pub fn call_i32_export(&mut self, name: &str) -> Result<i32, HostError> {
        let f: TypedFunc<(), i32> = self
            .instance
            .get_typed_func(&mut self.store, name)
            .map_err(|_| HostError::MissingExport("i32-export-0"))?;
        self.arm_bounds();
        f.call(&mut self.store, ()).map_err(Self::map_trap)
    }

    pub fn write_guest(&mut self, ptr: u32, bytes: &[u8]) -> Result<(), HostError> {
        crate::memory::write_bytes(&mut self.store, &self.memory, ptr, bytes)
    }

    pub fn read_return_buffer_copy(&mut self) -> Result<Vec<u8>, HostError> {
        Ok(self.store.data().host.return_buffer.as_slice().to_vec())
    }

    pub fn call_i32_export_2(&mut self, name: &str, a: i32, b: i32) -> Result<i32, HostError> {
        let f: TypedFunc<(i32, i32), i32> = self
            .instance
            .get_typed_func(&mut self.store, name)
            .map_err(|_| HostError::MissingExport("i32-export-2"))?;
        self.arm_bounds();
        f.call(&mut self.store, (a, b)).map_err(Self::map_trap)
    }

    pub fn call_i64_export_1(&mut self, name: &str, arg: i32) -> Result<i64, HostError> {
        let f: TypedFunc<i32, i64> = self
            .instance
            .get_typed_func(&mut self.store, name)
            .map_err(|_| HostError::MissingExport("i64-export-1"))?;
        self.arm_bounds();
        f.call(&mut self.store, arg).map_err(Self::map_trap)
    }
}

impl HostRuntime {
    /// §18.4 serve flow for content. Treats request/response as opaque bytes;
    /// the host NEVER decrypts or inspects the payload.
    pub fn serve_content(&mut self, request: &[u8]) -> Result<Vec<u8>, HostError> {
        self.serve_via("get_content", request)
    }

    /// §18.4 serve flow for proofs.
    pub fn serve_proof(&mut self, request: &[u8]) -> Result<Vec<u8>, HostError> {
        self.serve_via("get_proof", request)
    }

    fn serve_via(&mut self, export: &'static str, request: &[u8]) -> Result<Vec<u8>, HostError> {
        // 1. alloc(req_len) — bounds armed per sub-call (§18.2).
        let alloc: TypedFunc<i32, i32> = self
            .instance
            .get_typed_func(&mut self.store, "alloc")
            .map_err(|_| HostError::MissingExport("alloc"))?;
        self.arm_bounds();
        let req_ptr = alloc
            .call(&mut self.store, request.len() as i32)
            .map_err(Self::map_trap)?;

        // 2. write request bytes
        crate::memory::write_bytes(&mut self.store, &self.memory, req_ptr as u32, request)?;

        // 3. call get_content/get_proof(ptr, len)
        let serve: TypedFunc<(i32, i32), i64> = self
            .instance
            .get_typed_func(&mut self.store, export)
            .map_err(|_| HostError::MissingExport(export))?;
        self.arm_bounds();
        let packed = serve
            .call(&mut self.store, (req_ptr, request.len() as i32))
            .map_err(Self::map_trap)?;

        // 4-5. is_error? else unpack + read
        let out = self.unpack_and_read(packed);

        // 6. dealloc(req_ptr, req_len) — best effort.
        if let Ok(dealloc) = self
            .instance
            .get_typed_func::<(i32, i32), ()>(&mut self.store, "dealloc")
        {
            self.arm_bounds();
            let _ = dealloc.call(&mut self.store, (req_ptr, request.len() as i32));
        }

        out
    }
}
