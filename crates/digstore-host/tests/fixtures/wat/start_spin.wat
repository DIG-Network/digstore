;; A hostile module whose wasm `start` function never returns. Instantiating it
;; MUST fail promptly (fuel/epoch bounded) rather than hang the host: the
;; sandbox budget covers instantiation, not just exported calls.
(module
  (memory (export "memory") 1 256)
  (func $start (loop $l (br $l)))
  (start $start)
  (func (export "alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "dealloc") (param i32) (param i32))
  (func (export "init") (result i32) (i32.const 0))
  (func (export "get_store_id") (result i64) (i64.const 0))
)
