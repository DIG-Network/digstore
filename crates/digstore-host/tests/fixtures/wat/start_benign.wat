;; A well-behaved module with a wasm `start` function that does a small, bounded
;; amount of work before any export is called. Instantiating it MUST succeed:
;; the host arms a real execution budget around instantiation, not a zero one.
(module
  (memory (export "memory") 1 256)
  (global $bump (mut i32) (i32.const 1024))
  ;; start: fill 32 bytes at ptr 256 with 0xAB (bounded, ~32 iterations).
  (func $start
    (local $i i32)
    (local.set $i (i32.const 0))
    (block $done
      (loop $l
        (br_if $done (i32.ge_u (local.get $i) (i32.const 32)))
        (i32.store8 (i32.add (i32.const 256) (local.get $i)) (i32.const 0xAB))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $l))))
  (start $start)
  (func (export "alloc") (param $size i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $bump))
    (global.set $bump (i32.add (global.get $bump) (local.get $size)))
    (local.get $ptr))
  (func (export "dealloc") (param $ptr i32) (param $size i32))
  (func (export "init") (result i32) (i32.const 0))
  ;; Returns pack_ptr_len(256, 32) over the bytes `start` wrote, so a successful
  ;; read proves the start function actually ran to completion.
  (func (export "get_store_id") (result i64)
    (i64.or (i64.shl (i64.const 256) (i64.const 32)) (i64.const 32)))
)
