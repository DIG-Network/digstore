(module
  (memory (export "memory") 1 256)
  (func (export "alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "dealloc") (param i32) (param i32))
  (func (export "init") (result i32) (i32.const 0))
  ;; get_store_id: try to grow by 200 pages; if grow returns -1, trap.
  (func (export "get_store_id") (result i64)
    (if (i32.eq (memory.grow (i32.const 200)) (i32.const -1))
      (then (unreachable)))
    (i64.const 0))
)
