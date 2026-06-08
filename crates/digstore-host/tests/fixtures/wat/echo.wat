(module
  (memory (export "memory") 1 256)
  (global $bump (mut i32) (i32.const 1024))
  (func (export "alloc") (param $size i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $bump))
    (global.set $bump (i32.add (global.get $bump) (local.get $size)))
    (local.get $ptr))
  (func (export "dealloc") (param $ptr i32) (param $size i32))
  (func (export "init") (result i32) (i32.const 0))
  ;; get_store_id: writes 32 bytes of 0xAB at ptr 256, returns pack_ptr_len(256, 32).
  (func (export "get_store_id") (result i64)
    (local $i i32)
    (local.set $i (i32.const 0))
    (block $done
      (loop $l
        (br_if $done (i32.ge_u (local.get $i) (i32.const 32)))
        (i32.store8 (i32.add (i32.const 256) (local.get $i)) (i32.const 0xAB))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $l)))
    (i64.or (i64.shl (i64.const 256) (i64.const 32)) (i64.const 32)))
)
