(module
  (memory (export "memory") 1 256)
  (func (export "alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "dealloc") (param i32) (param i32))
  (func (export "init") (result i32) (i32.const 0))
  (func (export "get_store_id") (result i64)
    (loop $l (br $l))
    (i64.const 0))
)
