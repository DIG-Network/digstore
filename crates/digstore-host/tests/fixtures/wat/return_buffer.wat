(module
  (import "dig_host" "host_random_bytes" (func $hrb (param i32) (result i32)))
  (import "dig_host" "host_read_return_buffer" (func $hrr (param i32) (result i32)))
  (memory (export "memory") 8 256)
  (func (export "alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "dealloc") (param i32) (param i32))
  (func (export "init") (result i32) (i32.const 0))
  ;; fill_and_read(n): random(n) -> buffer; copy buffer into mem@131072; return packed.
  (func (export "fill_and_read") (param $n i32) (result i64)
    (local $w i32) (local $copied i32)
    (local.set $w (call $hrb (local.get $n)))
    (if (i32.lt_s (local.get $w) (i32.const 0))
      (then (return (i64.shl (i64.extend_i32_s (local.get $w)) (i64.const 32)))))
    (local.set $copied (call $hrr (i32.const 131072)))
    (i64.or
      (i64.shl (i64.const 131072) (i64.const 32))
      (i64.extend_i32_u (local.get $copied))))
)
