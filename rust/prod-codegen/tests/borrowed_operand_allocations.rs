//! Read-only uses of an owned local must not clone its allocation.

use prod_codegen::{
    generate_cargo_package, generate_core_wasm_package, CargoPackageSpec, CoreWasmSpec,
};
use prod_ir::parser::parse_module;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::Command;

const IR: &str = r#"(module BorrowedOperands
  (type "Parcel" (ctor "Parcel.mk" (bytes Bytes) (offset Nat)))
  (type "Fields" (ctor "Fields.mk" (bytes Bytes) (text String) (words (List String)) (offset Nat)))
  (type "Pair" (ctor "Pair.mk" (first Bytes) (second Bytes)))
  (type "CopyTag" (ctor "CopyTag.first") (ctor "CopyTag.second"))
  (def decode_field ((input (named "Parcel"))) (Option String)
    (utf8-decode (proj "Parcel" "bytes" input)))
  (def decode_alias ((input (named "Parcel"))) (Option String)
    (let bytes (proj "Parcel" "bytes" input) (utf8-decode bytes)))
  (def decode_owned ((input Bytes)) (Option String) (utf8-decode input))
  (def empty_byte_length () Nat (length (bytes)))
  (def empty_byte_index () (Option UInt8) (index (bytes) 0))
  (def empty_byte_slice () (Option Bytes) (slice (bytes) 0 0))
  (def decode_field_entry ((input Bytes)) Bytes
    (let owner (ctor "Parcel.mk" input 0)
      (cases (call decode_alias owner)
        (alt "Option.none" () (bytes 255))
        (alt "Option.some" (text)
          (if (eq (length (proj "Parcel" "bytes" owner)) (length text))
            (utf8-encode text) (bytes 254))))))
  (def equal_tag ((left (named "CopyTag")) (right (named "CopyTag"))) Bool (eq left right))
  (def inspect_error ((input (Result Bytes (named "CopyTag"))) (expected (named "CopyTag"))) Bool
    (cases input
      (alt "Except.ok" (bytes) (eq (length bytes) 8192))
      (alt "Except.error" (tag) (call equal_tag tag expected))))
  (def copy_pattern_entry ((input Bytes)) Bytes
    (if (call inspect_error (ctor "Except.error" (ctor "CopyTag.second")) (ctor "CopyTag.second")) input (bytes)))
  (def move_fields ((input (Option (named "Fields")))) (Option (named "Fields"))
    (cases input
      (alt "Option.none" () (ctor "Option.none"))
      (alt "Option.some" (row)
        (let bytes (proj "Fields" "bytes" row)
          (let text (proj "Fields" "text" row)
            (let words (proj "Fields" "words" row)
              (let offset (proj "Fields" "offset" row)
                (ctor "Option.some" (ctor "Fields.mk" bytes text words offset)))))))))
  (def duplicate_moved_alias ((input (Option (named "Parcel")))) (Option (named "Pair"))
    (cases input
      (alt "Option.none" () (ctor "Option.none"))
      (alt "Option.some" (row)
        (let bytes (proj "Parcel" "bytes" row)
          (ctor "Option.some" (ctor "Pair.mk" bytes bytes))))))
  (def borrowed_field ((input (named "Parcel"))) (Option Bytes)
    (ctor "Option.some" (proj "Parcel" "bytes" input)))
  (def accessor ((input (named "Parcel"))) Bytes
    (let result (proj "Parcel" "bytes" input) result))
  (def owned_result_choice ((left (named "Parcel")) (right (named "Parcel")) (flag Bool)) Bytes
    (cases flag
      (alt "Bool.false" () (let result (proj "Parcel" "bytes" left) result))
      (alt "Bool.true" () (let result (proj "Parcel" "bytes" right) result))))
  (def owned_result_entry ((input Bytes)) Bytes
    (let owner (ctor "Parcel.mk" input 0)
      (call owned_result_choice owner owner true)))
  (def mixed_fields ((input Bytes) (other (named "Parcel")) (choose Bool)) (Option Bytes)
    (let row (ctor "Parcel.mk" input 0)
      (let selected (if choose (proj "Parcel" "bytes" row) (proj "Parcel" "bytes" other))
        (ctor "Option.some" selected))))
  (def shadowed_fields ((row Bytes)) (Option Bytes)
    (let row (ctor "Parcel.mk" row 0)
      (let row (proj "Parcel" "bytes" row) (ctor "Option.some" row))))
  (def owned_field_entry ((input Bytes)) Bytes
    (cases (call consume_parcel (call fresh input))
      (alt "Option.none" () (bytes))
      (alt "Option.some" (result) result)))
  (def maybe_bytes ((input Bytes) (present Bool)) (Option Bytes)
    (if present (ctor "Option.some" input) (ctor "Option.none")))
  (def none_return ((input Bytes) (present Bool)) (Option Bytes)
    (let result (call maybe_bytes input present)
      (cases result
        (alt "Option.none" () result)
        (alt "Option.some" (value) (ctor "Option.some" value)))))
  (def none_return_shadow ((input Bytes) (present Bool)) (Option Bytes)
    (let result (call maybe_bytes input present)
      (cases result
        (alt "Option.some" (result) (ctor "Option.some" result))
        (alt "Option.none" () result))))
  (def none_return_shadow_reuse ((input Bytes) (present Bool)) (Option Bytes)
    (let result (call maybe_bytes input present)
      (cases result
        (alt "Option.none" () result)
        (alt "Option.some" (result)
          (ctor "Option.some" (append (call own_bytes result) result))))))
  (def none_return_shadow_join ((input Bytes) (present Bool)) (Option Bytes)
    (let result (call maybe_bytes input present)
      (cases result
        (alt "Option.none" () result)
        (alt "Option.some" (result)
          (let continuation (jp copy () result)
            (ctor "Option.some" (append (jmp copy) (jmp copy))))))))
  (def none_return_keeps_owner ((input Bytes) (present Bool)) (Option Bytes)
    (let result (call maybe_bytes input present)
      (cases result
        (alt "Option.none" () result)
        (alt "Option.some" (value)
          (cases result
            (alt "Option.none" () (ctor "Option.none"))
            (alt "Option.some" (later) (ctor "Option.some" (append value later))))))))
  (def none_return_type_constraint ((input Bytes) (present Bool)) Nat
    (let selected
      (let result (call maybe_bytes input present)
        (cases result
          (alt "Option.none" () result)
          (alt "Option.some" (value) (ctor "Option.none"))))
      (cases selected
        (alt "Option.none" () 0)
        (alt "Option.some" (value) 1))))
  (def none_return_builder ((input Bytes) (present Bool)) (List Nat)
    (let selected
      (let result (call maybe_bytes input present)
        (cases result
          (alt "Option.none" () result)
          (alt "Option.some" (value) (ctor "Option.some" value))))
      (cases selected
        (alt "Option.none" () (ctor "List.nil"))
        (alt "Option.some" (bytes) (ctor "List.cons" (length bytes) (ctor "List.nil"))))))
  (def fresh ((input Bytes)) (Option (named "Parcel"))
    (ctor "Option.some" (ctor "Parcel.mk" input 0)))
  (def parcel ((input Bytes)) (Option Bytes)
    (let result (call fresh input)
      (cases result
        (alt "Option.none" () (ctor "Option.none"))
        (alt "Option.some" (row)
          (let bytes (proj "Parcel" "bytes" row)
            (let width (length bytes)
              (let offset (proj "Parcel" "offset" row)
                (if (eq width offset)
                    (ctor "Option.some" (proj "Parcel" "bytes" row))
                    (ctor "Option.some" bytes)))))))))
  (def accepts ((input Bytes)) Bool (eq (length input) 8192))
  (def predicate ((input Bytes)) (Option Bytes)
    (if (call accepts input) (ctor "Option.some" input) (ctor "Option.none")))
  (def own_bytes ((input Bytes)) Bytes input)
  (def own_text ((input String)) String input)
  (def branch_parameter ((input Bytes) (choose Bool)) Bytes
    (if choose input input))
  (def branch_alias ((input Bytes) (choose Bool)) Bytes
    (let local (call own_bytes input) (if choose local local)))
  (def branch_nested ((input Bytes) (first Bool) (second Bool)) Bytes
    (cases first
      (alt "Bool.false" () (if second input input))
      (default (if second input input))))
  (def branch_join ((input Bytes) (choose Bool)) Bytes
    (let finish (jp finish (value) (if choose value value))
      (jmp finish input)))
  (def branch_text ((input String) (choose Bool)) String
    (if choose input input))
  (def branch_walk ((remaining Nat) (input Bytes)) Bytes
    (if (eq remaining 0) input (call branch_walk (sub remaining 1) input)))
  (def branch_append ((remaining Nat) (input Bytes) (suffix (named "Parcel"))) Bytes
    (if (eq remaining 0) input
      (call branch_append (sub remaining 1) (append input (proj "Parcel" "bytes" suffix)) suffix)))
  (def branch_join_retained ((input Bytes) (choose Bool)) (named "Pair")
    (let finish (jp finish (value) (if choose value value))
      (let first (jmp finish input)
        (ctor "Pair.mk" first (jmp finish input)))))
  (def branch_retained ((input Bytes) (choose Bool)) (named "Pair")
    (let first (if choose (call own_bytes input) (call own_bytes input))
      (ctor "Pair.mk" first input)))
  (def branch_duplicate ((input Bytes) (choose Bool)) (named "Pair")
    (if choose (ctor "Pair.mk" input input) (ctor "Pair.mk" input (bytes))))
  (def branch_scrutinee ((input Bytes)) Bytes
    (cases (utf8-decode input)
      (alt "Option.none" () input)
      (alt "Option.some" (text) (append input (utf8-encode text)))))
  (def branch_eager_failure ((input Bytes) (choose Bool) (maximum Nat)) Bytes
    (let unused (add maximum 1) (if choose input input)))
  (def branch_entry ((input Bytes)) Bytes (call branch_walk 64 input))
  (def slice_read ((input Bytes)) (Option Bytes)
    (let local (call own_bytes input)
      (let part (slice local 0 1)
        (if (eq (length local) 0) (ctor "Option.none") part))))
  (def index_read ((input Bytes)) (Option UInt8)
    (let local (call own_bytes input)
      (let first (index local 0)
        (if (eq (length local) 0) (ctor "Option.none") first))))
  (def append_right ((input Bytes)) Bytes
    (let local (call own_bytes input)
      (if (eq (length local) 0) (bytes) (append (bytes) local))))
  (def self_append ((input Bytes)) Bytes (append input input))
  (def length_then_move ((input Bytes)) (named "Parcel")
    (let width (length input) (ctor "Parcel.mk" input width)))
  (def scalar_length_then_move ((input String)) String
    (let width (string-length input) (if (eq width 0) input input)))
  (def move_then_length ((input Bytes)) (named "Pair")
    (let moved (call own_bytes input)
      (let width (length input) (ctor "Pair.mk" moved (if (eq width 0) input input)))))
  (def move_then_last_length ((input Bytes)) (named "Parcel")
    (let moved (call own_bytes input)
      (let width (length input) (ctor "Parcel.mk" moved width))))
  (def read_move_read ((input Bytes)) (named "Parcel")
    (let before (length input)
      (let moved (call own_bytes input)
        (let after (length input) (ctor "Parcel.mk" moved after)))))
  (def length_entry ((input Bytes)) Bytes
    (let width (length input) (if (eq width 0) input input)))
  (def branch_length_guarded ((input Bytes) (suffix Bytes)) Bytes
    (let first (length input)
      (if (le first 1048576)
        (let second (length suffix)
          (if (le second 1048576)
            (let third (length input)
              (let fourth (length suffix)
                (if (le (add third fourth) 2097152)
                  (append input suffix) (bytes))))
            (bytes)))
        (bytes))))
  (def branch_length_match ((input Bytes) (tag (named "CopyTag"))) Bytes
    (cases tag
      (alt "CopyTag.first" ()
        (let width (length input) (if (eq width 0) input input)))
      (default
        (let width (length input) (if (eq width 0) input input)))))
  (def branch_length_local ((input Bytes) (choose Bool)) Bytes
    (let local (call own_bytes input)
      (if choose (let width (length local) (if (eq width 0) local local))
        (let width (length local) local))))
  (def branch_scalar_length ((input String) (choose Bool)) String
    (if choose (let width (string-length input) (if (eq width 0) input input))
      (let width (string-length input) input)))
  (def branch_length_entry ((input Bytes)) Bytes
    (call branch_length_guarded input (bytes)))
  (def branch_move_then_read ((input Bytes) (choose Bool)) (named "Parcel")
    (if choose
      (let before (length input)
        (let moved (call own_bytes input)
          (let after (length input) (ctor "Parcel.mk" moved after))))
      (let moved (call own_bytes input)
        (let after (length input) (ctor "Parcel.mk" moved after)))))
  (def branch_then_outer_read ((input Bytes) (choose Bool)) (named "Parcel")
    (let moved
      (if choose (let width (length input) input)
        (let width (length input) input))
      (ctor "Parcel.mk" moved (length input))))
  (def branch_retained_length_alias ((input Bytes) (choose Bool)) (named "Pair")
    (if choose
      (let alias input
        (let width (length input) (ctor "Pair.mk" alias input)))
      (let width (length input) (ctor "Pair.mk" input input))))
  (def consuming_predicate ((input Bytes)) Bool
    (let moved (call own_bytes input) (eq (length moved) 0)))
  (def branch_consuming_condition ((input Bytes)) Bytes
    (if (call consuming_predicate input)
      (let width (length input) input)
      (let width (length input) input)))
  (def branch_consuming_scrutinee ((input Bytes)) Bytes
    (cases (call own_bytes input)
      (default (let width (length input) input))))
  (def branch_unknown_call ((input Bytes) (choose Bool)) Bytes
    (if choose
      (let observed (call consuming_predicate input)
        (let width (length input) input))
      (let width (length input) input)))
  (def branch_retained_projection ((input (named "Parcel")) (choose Bool)) (named "Pair")
    (if choose
      (let alias (proj "Parcel" "bytes" input)
        (let width (length alias)
          (ctor "Pair.mk" alias (proj "Parcel" "bytes" input))))
      (ctor "Pair.mk" (proj "Parcel" "bytes" input) (proj "Parcel" "bytes" input))))
  (def self_append_retained ((input Bytes)) (named "Pair")
    (let twice (append input input) (ctor "Pair.mk" twice input)))
  (def self_append_borrowed ((input (named "Parcel"))) Bytes
    (let bytes (proj "Parcel" "bytes" input) (append bytes bytes)))
  (def self_append_hygiene ((__value Bytes)) Bytes (append __value __value))
  (def self_append_words ((input (Option (List String)))) (Option (List String))
    (cases input
      (alt "Option.none" () (ctor "Option.none"))
      (alt "Option.some" (words) (ctor "Option.some" (append words words)))))
  (def compare_read ((input Bytes) (other Bytes)) Ordering
    (let left (call own_bytes input)
      (let right (call own_bytes other)
        (let compared (compare-bytes left right)
          (if (eq (length left) (length right)) compared compared)))))
  (def equality_read ((input Bytes)) Bool
    (let local (call own_bytes input)
      (if (eq local input) (eq local (bytes)) false)))
  (def fresh_words ((input (List String))) (Option (List String))
    (ctor "Option.some" input))
  (def prepend_owned ((head String) (tail (Option (List String)))) (Option (List String))
    (cases tail
      (alt "Option.none" () (ctor "Option.none"))
      (alt "Option.some" (values)
        (ctor "Option.some" (ctor "List.cons" head values)))))
  (def prepend_borrowed ((head String) (tail (List String))) (Option (List String))
    (ctor "Option.some" (ctor "List.cons" head tail)))
  (def prepend_retained ((head String) (tail (Option (List String)))) (Option (List String))
    (cases tail
      (alt "Option.none" () (ctor "Option.none"))
      (alt "Option.some" (values)
        (let prefixed (ctor "List.cons" head values)
          (ctor "Option.some" (append prefixed values))))))
  (def prepend_failure_order ((head Nat) (tail Nat)) (Option (List Nat))
    (ctor "Option.some"
      (ctor "List.cons" (add head 1)
        (ctor "List.cons" (mul tail 2) (ctor "List.nil")))))
  (def prepend_temporary ((head String) (__list (Option (List String)))) (Option (List String))
    (cases __list
      (alt "Option.none" () (ctor "Option.none"))
      (alt "Option.some" (values)
        (ctor "Option.some" (ctor "List.cons" head values)))))
  (def owned_list_entry ((input Bytes)) Bytes
    (cases (utf8-decode input)
      (alt "Option.none" () (bytes))
      (alt "Option.some" (text)
        (cases (call prepend_owned (string "head") (split-exact text (string "|") 8192))
          (alt "Option.none" () (bytes))
          (alt "Option.some" (values) (utf8-encode (join values (string "|"))))))))
  (def join_read ((input (List String)) (separator String)) (Option String)
    (cases (call fresh_words input)
      (alt "Option.none" () (ctor "Option.none"))
      (alt "Option.some" (words)
        (let delimiter (call own_text separator)
          (let joined (join words delimiter)
            (if (eq (length words) 0) (ctor "Option.none")
              (if (eq (length delimiter) 2)
                (ctor "Option.some" joined) (ctor "Option.none"))))))))
  (def decimal_read ((input String)) (Option UInt16)
    (let local (call own_text input)
      (let parsed (parse-decimal-as UInt16 local)
        (if (eq local (string "")) (ctor "Option.none") parsed))))
  (def consume_parcel ((value (Option (named "Parcel")))) (Option Bytes)
    (cases value
      (alt "Option.none" () (ctor "Option.none"))
      (alt "Option.some" (row) (ctor "Option.some" (proj "Parcel" "bytes" row)))))
  (def projection_then_consume ((input Bytes)) (Option Bytes)
    (let owner (ctor "Parcel.mk" input 0)
      (let viewed (proj "Parcel" "bytes" owner)
        (let offset (proj "Parcel" "offset" owner)
          (let consumed (call consume_parcel (ctor "Option.some" owner))
            (if (eq (length viewed) offset) (ctor "Option.none") consumed))))))
  (def entry ((input Bytes)) Bytes
    (cases (call none_return input true)
      (alt "Option.none" () (bytes))
      (alt "Option.some" (output)
        (cases (call parcel output)
          (alt "Option.none" () (bytes))
          (alt "Option.some" (result) result))))))"#;

const RUNNER: &str = r#"use borrowed_operands_fixture::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counted;
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counted = Counted;

fn measured<T>(action: impl FnOnce() -> T) -> (T, usize) {
    ALLOCATIONS.store(0, Ordering::Relaxed);
    let value = action();
    (value, ALLOCATIONS.load(Ordering::Relaxed))
}

fn main() {
    for size in [0, 1, 32, 8192, 1_048_576] {
        let mut input = Vec::with_capacity(size + 1);
        input.resize(size, 17);
        let pointer = input.as_ptr();
        let suffix = vec![9];
        let (output, count) = measured(|| branch_length_guarded(input, suffix).unwrap());
        assert_eq!(count, 0, "branch-local scalar guards must not clone an owned append operand");
        assert_eq!(output.as_ptr(), pointer);
        assert_eq!(&output[..size], vec![17; size]);
        assert_eq!(output[size], 9);
        for choose in [false, true] {
            let input = vec![17; size];
            let pointer = input.as_ptr();
            let tag = if choose { CopyTag::first } else { CopyTag::second };
            let (output, count) = measured(|| branch_length_match(input, tag));
            assert_eq!(count, 0, "scalar reads in exclusive match branches retain no owner");
            assert_eq!(output.as_ptr(), pointer);
            assert_eq!(output, vec![17; size]);
            let input = vec![17; size];
            let pointer = input.as_ptr();
            let (output, count) = measured(|| branch_length_local(input, choose));
            assert_eq!(count, 0, "branch-local scalar reads of a local retain no owner");
            assert_eq!(output.as_ptr(), pointer);
            assert_eq!(output, vec![17; size]);
            let input = "é".repeat(size);
            let pointer = input.as_ptr();
            let (output, count) = measured(|| branch_scalar_length(input, choose));
            assert_eq!(count, 0, "branch-local scalar-count reads retain no owner");
            assert_eq!(output.as_ptr(), pointer);
            assert_eq!(output, "é".repeat(size));
            for action in [branch_move_then_read, branch_then_outer_read] {
                let output = action(vec![17; size], choose);
                assert_eq!(output.bytes, vec![17; size]);
                assert_eq!(output.offset, size as u64, "a branch cannot hide a post-transfer read");
            }
            let mut output = branch_retained_length_alias(vec![17; size], choose);
            assert_eq!(output.first, vec![17; size]);
            assert_eq!(output.second, vec![17; size]);
            output.first.push(9);
            assert_eq!(output.second.len(), size, "a retained alias needs an independent owner");
            let original = Parcel { bytes: vec![17; size], offset: 7 };
            let mut output = branch_retained_projection(&original, choose);
            assert_eq!(output.first, original.bytes);
            assert_eq!(output.second, original.bytes);
            output.first.push(9);
            assert_eq!(output.second.len(), size);
            assert_eq!(original.bytes.len(), size, "borrowed projections retain their owner");
            assert_eq!(branch_unknown_call(vec![17; size], choose), vec![17; size]);
        }
        assert_eq!(branch_consuming_condition(vec![17; size]), vec![17; size]);
        assert_eq!(branch_consuming_scrutinee(vec![17; size]), vec![17; size]);
        let input = vec![17; size];
        let pointer = input.as_ptr();
        let (output, count) = measured(|| length_then_move(input));
        assert_eq!(count, 0, "a completed scalar read cannot retain ownership");
        assert_eq!(output.bytes.as_ptr(), pointer);
        assert_eq!(output.offset, size as u64);
        assert_eq!(output.bytes, vec![17; size]);
        let text = "é".repeat(size);
        let pointer = text.as_ptr();
        let (output, count) = measured(|| scalar_length_then_move(text));
        assert_eq!(count, 0, "a completed scalar-count read cannot retain ownership");
        assert_eq!(output.as_ptr(), pointer);
        assert_eq!(output, "é".repeat(size));
        for append in [self_append, self_append_hygiene] {
            let mut input = Vec::with_capacity(size * 2);
            input.resize(size, 17);
            let pointer = input.as_ptr();
            let (output, count) = measured(|| append(input));
            assert_eq!(count, 0, "self append reuses sufficient owned capacity");
            assert_eq!(output.as_ptr(), pointer);
            assert_eq!(output, vec![17; size * 2]);
        }
        let output = move_then_length(vec![17; size]);
        assert_eq!(output.first, vec![17; size]);
        assert_eq!(output.second, vec![17; size]);
        for action in [move_then_last_length, read_move_read] {
            let output = action(vec![17; size]);
            assert_eq!(output.bytes, vec![17; size]);
            assert_eq!(output.offset, size as u64, "later length reads must retain their owner");
        }
        let mut output = self_append_retained(vec![17; size]);
        assert_eq!(output.first, vec![17; size * 2]);
        assert_eq!(output.second, vec![17; size]);
        output.first.push(9);
        assert_eq!(output.second.len(), size);
        let original = Parcel { bytes: vec![17; size], offset: 0 };
        assert_eq!(self_append_borrowed(&original), vec![17; size * 2]);
        assert_eq!(original.bytes, vec![17; size]);
    }
    let mut output = self_append_words(Some(vec!["é".into(), "tail".into()])).unwrap();
    assert_eq!(output, vec!["é", "tail", "é", "tail"]);
    output[0].push('!');
    assert_eq!(output[2], "é");
    assert_eq!(self_append_words(None), None);
    for text in ["", "ascii", "\0", "é", "e\u{301}", "🦀", "日本語"] {
        for decode in [decode_field, decode_alias] {
            let input = Parcel { bytes: text.as_bytes().to_vec(), offset: 7 };
            let pointer = input.bytes.as_ptr();
            let (output, count) = measured(|| decode(&input));
            assert_eq!(output.as_deref(), Some(text));
            assert_eq!(count, usize::from(!text.is_empty()), "borrowed valid UTF-8 allocates only its owned result");
            assert_eq!(input.bytes.as_ptr(), pointer);
            assert_eq!(input.bytes, text.as_bytes());
        }
        let input = text.as_bytes().to_vec();
        let pointer = input.as_ptr();
        let (output, count) = measured(|| decode_owned(input));
        assert_eq!(count, 0, "owned UTF-8 decoding must reuse the input buffer");
        let output = output.unwrap();
        assert_eq!(output, text);
        assert_eq!(output.as_ptr(), pointer);
    }
    for bytes in [vec![255], vec![0xc0, 0x80], vec![0xed, 0xa0, 0x80], vec![0xf4, 0x90, 0x80, 0x80], vec![0xe2, 0x82]] {
        let input = Parcel { bytes, offset: 3 };
        for decode in [decode_field, decode_alias] {
            let (output, count) = measured(|| decode(&input));
            assert_eq!(output, None);
            assert_eq!(count, 0, "invalid borrowed UTF-8 must not allocate a copy");
        }
        let (output, count) = measured(|| decode_owned(input.bytes));
        assert_eq!(output, None);
        assert_eq!(count, 0);
    }
    let (output, count) = measured(|| (empty_byte_length(), empty_byte_index(), empty_byte_slice()));
    assert_eq!(output, (0, None, Some(Vec::new())));
    assert_eq!(count, 0, "typed empty byte literals allocate nothing");
    for expected in [CopyTag::first, CopyTag::second] {
        for actual in [CopyTag::first, CopyTag::second] {
            let (output, count) = measured(|| inspect_error(Err(actual), expected));
            assert_eq!(output, actual == expected);
            assert_eq!(count, 0, "Copy pattern payloads do not allocate");
        }
        let input = vec![7; 8192];
        let (output, count) = measured(|| inspect_error(Ok(input), expected));
        assert!(output);
        assert_eq!(count, 0, "non-Copy alternative remains borrowed");
    }
    for size in [0, 1, 32, 8192] {
        for choose in [false, true] {
            for action in [branch_parameter, branch_alias, branch_join] {
                let input = vec![19; size];
                let pointer = input.as_ptr();
                let (output, count) = measured(|| action(input, choose));
                assert_eq!(count, 0, "exclusive owned branch must move, size={size}");
                assert_eq!(output.as_ptr(), pointer);
                assert_eq!(output, vec![19; size]);
            }
            for second in [false, true] {
                let input = vec![19; size];
                let pointer = input.as_ptr();
                let (output, count) = measured(|| branch_nested(input, choose, second));
                assert_eq!(count, 0, "nested/default alternatives share no execution path");
                assert_eq!(output.as_ptr(), pointer);
                assert_eq!(output, vec![19; size]);
            }
            let input = "x".repeat(size);
            let pointer = input.as_ptr();
            let (output, count) = measured(|| branch_text(input, choose));
            assert_eq!(count, 0, "String ownership is branch-exclusive too");
            assert_eq!(output.as_ptr(), pointer);
            assert_eq!(output, "x".repeat(size));

            let input = vec![19; size];
            let mut output = branch_retained(input, choose);
            assert_eq!(output.first, vec![19; size]);
            assert_eq!(output.second, vec![19; size]);
            if size > 0 { output.first[0] = 23; assert_eq!(output.second[0], 19); }
            let mut output = branch_duplicate(vec![19; size], choose);
            assert_eq!(output.first, vec![19; size]);
            assert_eq!(output.second, vec![19; if choose { size } else { 0 }]);
            if choose && size > 0 { output.first[0] = 23; assert_eq!(output.second[0], 19); }
            let mut output = branch_join_retained(vec![19; size], choose);
            assert_eq!(output.first, vec![19; size]);
            assert_eq!(output.second, vec![19; size]);
            if size > 0 { output.first[0] = 23; assert_eq!(output.second[0], 19); }
            assert_eq!(branch_eager_failure(vec![19; size], choose, u64::MAX),
                       Err(ComputeError::AddOverflow));
        }
        for steps in [0, 1, 64] {
            let input = vec![19; size];
            let pointer = input.as_ptr();
            let (output, count) = measured(|| branch_walk(steps, input));
            assert_eq!(count, 0, "recursive exclusive ownership transfer");
            assert_eq!(output.as_ptr(), pointer);
            assert_eq!(output, vec![19; size]);
            let mut input = Vec::with_capacity(size + steps as usize);
            input.resize(size, 19);
            let pointer = input.as_ptr();
            let suffix = Parcel { bytes: vec![7], offset: 0 };
            let (output, count) = measured(|| branch_append(steps, input, &suffix));
            assert_eq!(count, 0, "preallocated accumulator must retain its capacity");
            assert_eq!(output.as_ptr(), pointer);
            assert_eq!(&output[..size], vec![19; size]);
            assert_eq!(&output[size..], vec![7; steps as usize]);
        }
    }
    assert_eq!(branch_scrutinee(vec![255]), [255]);
    assert_eq!(branch_scrutinee(b"abc".to_vec()), b"abcabc");
    for size in [0, 1, 32, 1024] {
        let owner = Parcel { bytes: vec![19; size], offset: 7 };
        let original = owner.bytes.as_ptr();
        for flag in [false, true] {
            let (mut result, count) = measured(|| owned_result_choice(&owner, &owner, flag));
            assert_eq!(result, owner.bytes);
            assert_eq!(count, usize::from(size > 0), "exactly one owned-result copy");
            assert_eq!(owner.bytes.as_ptr(), original, "borrowed input allocation retained");
            if size > 0 {
                result[0] = 23;
                assert_eq!(owner.bytes[0], 19, "owned result cannot modify its source");
            }
        }
    }
    for size in [0, 1, 32, 1024] {
        let mut tail = Vec::with_capacity(size + 1);
        for index in 0..size {
            tail.push(format!("tail-{index}"));
        }
        let pointer = tail.as_ptr();
        let head = "head".to_owned();
        let mut expected = vec![head.clone()];
        expected.extend(tail.clone());
        let (output, count) = measured(|| prepend_owned(head, Some(tail)));
        let output = output.unwrap();
        assert_eq!(output, expected);
        assert_eq!(count, 0, "reuse owned tail capacity, size={size}");
        assert_eq!(output.as_ptr(), pointer, "retain owned tail allocation");

        let tail = expected[1..].to_vec().into_boxed_slice().into_vec();
        assert_eq!(tail.capacity(), tail.len(), "full tail fixture");
        let head = "head".to_owned();
        let (output, count) = measured(|| prepend_owned(head, Some(tail)));
        let output = output.unwrap();
        assert_eq!(output, expected);
        assert_eq!(count, 1, "one exact growth, size={size}");
        assert_eq!(output.capacity(), size + 1, "do not double full tail capacity");

        let tail = expected[1..].to_vec();
        let retained = tail.clone();
        let output = prepend_borrowed("head".to_owned(), &tail).unwrap();
        assert_eq!(output, expected);
        assert_eq!(tail, retained, "borrowed tail must remain unchanged");

        let head = "head".to_owned();
        let tail = retained.clone();
        let mut repeated = expected.clone();
        repeated.extend(retained);
        assert_eq!(prepend_retained(head, Some(tail)), Some(repeated));

        let tail = expected[1..].to_vec();
        assert_eq!(prepend_temporary("head".to_owned(), Some(tail)), Some(expected));
    }
    assert_eq!(prepend_owned("head".to_owned(), None), None);
    assert_eq!(prepend_failure_order(u64::MAX, u64::MAX), Err(ComputeError::AddOverflow));
    assert_eq!(prepend_failure_order(0, u64::MAX), Err(ComputeError::MulOverflow));
    assert_eq!(prepend_failure_order(0, 0), Ok(Some(vec![1, 0])));
    for size in [0, 1, 8192, 65536] {
        let input = vec![0x5a; size];
        let pointer = input.as_ptr();
        let (output, count) = measured(|| consume_parcel(Some(Parcel { bytes: input, offset: 7 })));
        assert_eq!(count, 0, "consuming an owned record field must not clone, size={size}");
        assert_eq!(output.as_ref().unwrap().as_ptr(), pointer);
        assert_eq!(output.as_deref(), Some(vec![0x5a; size].as_slice()));

        let input = Fields { bytes: vec![0x5a; size], text: "x".repeat(size),
            words: vec!["first".to_owned(), "second".to_owned()], offset: 13 };
        let pointers = (input.bytes.as_ptr(), input.text.as_ptr(), input.words.as_ptr(), input.words[0].as_ptr());
        let (output, count) = measured(|| move_fields(Some(input)));
        assert_eq!(count, 0, "disjoint owned fields, size={size}");
        let output = output.unwrap();
        assert_eq!((output.bytes.as_ptr(), output.text.as_ptr(), output.words.as_ptr(), output.words[0].as_ptr()), pointers);
        assert_eq!(output.bytes, vec![0x5a; size]);
        assert_eq!(output.text, "x".repeat(size));
        assert_eq!(output.words, ["first", "second"]);
        assert_eq!(output.offset, 13);

        let input = Parcel { bytes: vec![0x5a; size], offset: 0 };
        let output = duplicate_moved_alias(Some(input)).unwrap();
        assert_eq!(output.first, vec![0x5a; size]);
        assert_eq!(output.second, vec![0x5a; size]);
        if size != 0 { assert_ne!(output.first.as_ptr(), output.second.as_ptr()); }

        let input = Parcel { bytes: vec![0xa5; size], offset: 0 };
        let (output, count) = measured(|| borrowed_field(&input));
        assert_eq!(count, usize::from(size != 0), "borrowed record must retain its field");
        assert_eq!(output.as_deref(), Some(input.bytes.as_slice()));
        let (view, count) = measured(|| accessor(&input));
        assert_eq!(count, 0, "accessor ABI remains borrowed");
        assert!(std::ptr::eq(view, &input.bytes));
        for choose in [false, true] {
            let bytes = vec![0x5a; size];
            let pointer = bytes.as_ptr();
            let (output, count) = measured(|| mixed_fields(bytes, &input, choose));
            assert_eq!(count, usize::from(!choose && size != 0), "mixed ownership branches");
            assert_eq!(output.as_deref(), Some(vec![if choose { 0x5a } else { 0xa5 }; size].as_slice()));
            if choose { assert_eq!(output.as_ref().unwrap().as_ptr(), pointer); }
        }
        assert_eq!(input.bytes, vec![0xa5; size]);

        let input = vec![0x5a; size];
        let pointer = input.as_ptr();
        let (output, count) = measured(|| shadowed_fields(input));
        assert_eq!(count, 0, "lexically distinct owners must not alias");
        assert_eq!(output.as_ref().unwrap().as_ptr(), pointer);
        assert_eq!(output.as_deref(), Some(vec![0x5a; size].as_slice()));
        for present in [false, true] {
            let input = vec![0x5a; size];
            let (output, count) = measured(|| none_return(input, present));
            assert_eq!(count, 0, "None-arm return must not clone a Some payload, size={size}, present={present}");
            assert_eq!(output.as_deref(), present.then_some(vec![0x5a; size].as_slice()));

            let input = vec![0x5a; size];
            let (output, count) = measured(|| none_return_shadow(input, present));
            assert_eq!(count, 0, "reversed arms and shadowing, size={size}, present={present}");
            assert_eq!(output.as_deref(), present.then_some(vec![0x5a; size].as_slice()));

            let input = vec![0x5a; size];
            let (output, count) = measured(|| none_return_shadow_reuse(input, present));
            // Keep the old name-based protection for a shadowed payload that
            // is consumed by a call, then read again to append its bytes.
            assert_eq!(count, 3 * usize::from(present && size != 0), "shadowed payload reuse, size={size}, present={present}");
            assert_eq!(output.as_deref(), present.then_some(vec![0x5a; size * 2].as_slice()));

            let input = vec![0x5a; size];
            let (output, count) = measured(|| none_return_shadow_join(input, present));
            // fefc4ce's conservative owner analysis keeps the outer
            // `match result.clone()` (+1 allocation), while HEAD's in-place
            // self append moves the captured payload and extends it with
            // `extend_from_within` instead of cloning it first (one fewer
            // allocation than the pre-perf 3..=4 shape). The remaining
            // allocation is the extend's growth realloc.
            assert_eq!(count, 2 * usize::from(present && size != 0),
                    "captured shadowed payload reuse: allocations={count}, size={size}, present={present}");
            assert_eq!(output.as_deref(), present.then_some(vec![0x5a; size * 2].as_slice()));

            let input = vec![0x5a; size];
            let (output, count) = measured(|| none_return_keeps_owner(input, present));
            // A real owner use in the Some branch still needs the scrutinee
            // preserved; this conservative pass must not remove that clone.
            assert_eq!(count, 3 * usize::from(present && size != 0), "later owner use, size={size}, present={present}");
            assert_eq!(output.as_deref(), present.then_some(vec![0x5a; size * 2].as_slice()));

            let input = vec![0x5a; size];
            assert_eq!(none_return_type_constraint(input, present), 0);

            let input = vec![0x5a; size];
            let mut buffer = [99_u64; 1];
            let (output, count) = measured(|| none_return_builder(input, present, &mut buffer));
            assert_eq!(count, 0, "None return nested in builder, size={size}, present={present}");
            assert_eq!(output, Ok(usize::from(present)));
            assert_eq!(buffer[0], if present { size as u64 } else { 99 });
        }
        let input = vec![0x5a; size];
        ALLOCATIONS.store(0, Ordering::Relaxed);
        let output = parcel(input);
        let allocations = ALLOCATIONS.load(Ordering::Relaxed);
        assert_eq!(allocations, usize::from(size != 0),
                   "field reads must not clone the enclosing record, size={size}");
        assert_eq!(output.as_deref(), Some(vec![0x5a; size].as_slice()));

        let input = vec![0xa5; size];
        ALLOCATIONS.store(0, Ordering::Relaxed);
        let output = predicate(input);
        let allocations = ALLOCATIONS.load(Ordering::Relaxed);
        assert_eq!(allocations, usize::from(size == 8192),
                   "borrowed predicate argument must not be cloned, size={size}");
        assert_eq!(output.as_deref(), (size == 8192).then_some(vec![0xa5; size].as_slice()));

        let input = vec![0x5a; size];
        let (output, count) = measured(|| slice_read(input));
        // The returned one-byte slice owns one allocation; reading its source owns none.
        assert_eq!(count, usize::from(size != 0), "slice source, size={size}");
        assert_eq!(output.as_deref(), (size != 0).then_some([0x5a].as_slice()));

        let input = vec![0x5a; size];
        let (output, count) = measured(|| index_read(input));
        // The Copy-returning helper borrows input; own_bytes copies it once.
        assert_eq!(count, usize::from(size != 0), "index source, size={size}");
        assert_eq!(output, (size != 0).then_some(0x5a));

        let input = vec![0x5a; size];
        let (output, count) = measured(|| append_right(input));
        // Only the output buffer is allocated; the repeated right operand is borrowed.
        assert_eq!(count, usize::from(size != 0), "append right, size={size}");
        assert_eq!(output, vec![0x5a; size]);

        let input = vec![0x5a; size];
        let (output, count) = measured(|| self_append(input));
        // Self append grows the original buffer without an intermediate clone.
        assert_eq!(count, usize::from(size != 0), "self append, size={size}");
        assert_eq!(output, vec![0x5a; size * 2]);

        let input = vec![0x5a; size];
        let (output, count) = measured(|| equality_read(&input));
        assert_eq!(count, usize::from(size != 0), "mixed and literal equality, size={size}");
        assert_eq!(output, size == 0);

        let input = vec![0x5a; size];
        let (output, count) = measured(|| projection_then_consume(input));
        // Only the record needs copying to keep the original field borrow
        // live; consume_parcel moves the copied record's field into its result.
        assert_eq!(count, usize::from(size != 0), "project then consume, size={size}");
        assert_eq!(output.as_deref(), (size != 0).then_some(vec![0x5a; size].as_slice()));
    }

    for (left, right) in [(b"".as_slice(), b"".as_slice()), (b"a", b""),
                           (b"", b"a"), (b"a", b"b"), (b"b", b"a"), (b"ab", b"ab")] {
        let input = left.to_vec();
        let other = right.to_vec();
        let (output, count) = measured(|| compare_read(input, other));
        // Each borrowed input becomes one owned local; both comparisons borrow.
        assert_eq!(count, usize::from(!left.is_empty()) + usize::from(!right.is_empty()),
                   "compare bytes: {left:?}, {right:?}");
        assert_eq!(output, left.cmp(right));
    }

    for words in [vec![], vec!["one".to_owned()], vec!["one".to_owned(), "two".to_owned()]] {
        let expected = (!words.is_empty()).then(|| words.join("::"));
        let delimiter = "::".to_owned();
        let (output, count) = measured(|| join_read(&words, delimiter));
        // A nonempty list clone owns one vector plus its strings; the joined
        // output owns one String. The existing delimiter is moved, never cloned.
        let expected_count = if words.is_empty() { 0 } else { words.len() + 2 };
        assert_eq!(count, expected_count, "join words and delimiter: {words:?}");
        assert_eq!(output, expected);
    }

    for (text, expected, expected_count) in [
        ("", None, 0), ("0", Some(0), 2), ("256", Some(256), 2),
        ("65535", Some(65535), 2), ("0256", None, 2), ("65536", None, 1), ("x", None, 1),
    ] {
        let input = text.to_owned();
        let (output, count) = measured(|| decimal_read(input));
        // One nonempty local copy; successful parsing formats once to check canonical spelling.
        assert_eq!(count, expected_count, "decimal input: {text:?}");
        assert_eq!(output, expected);
    }
}
"#;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "prod-borrowed-operands-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("retained failing generated fixture: {}", self.0.display());
        } else {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
}

fn succeeds(command: &mut Command) {
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn read_only_owned_operands_do_not_allocate_in_std_and_no_std() {
    let (remaining, module) = parse_module(IR).unwrap();
    assert!(remaining.is_empty());
    let spec = CargoPackageSpec {
        name: "borrowed-operands-fixture".into(),
        version: "0.1.0".into(),
        description: "Borrowed operand allocation regression".into(),
        repository: "https://github.com/auser/lean4-prod".into(),
        homepage: "https://github.com/auser/lean4-prod".into(),
        readme: "Borrowed operand regression fixture.\n".into(),
        license_mit: "MIT\n".into(),
        license_apache: "Apache-2.0\n".into(),
        input_sha256: format!("{:x}", Sha256::digest(IR.as_bytes())),
        dependencies: vec![],
    };
    let package = generate_cargo_package(&module, &spec).unwrap();
    assert_eq!(package, generate_cargo_package(&module, &spec).unwrap());
    let scratch = Scratch::new();
    for file in package.files {
        let path = scratch.0.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.bytes).unwrap();
    }
    let runner = scratch.0.join("runner.rs");
    std::fs::write(&runner, RUNNER).unwrap();
    for standard in [true, false] {
        for optimized in [false, true] {
            let library = scratch
                .0
                .join(format!("libfixture_{standard}_{optimized}.rlib"));
            let mut compiler = Command::new("rustc");
            compiler.args([
                "--edition=2021",
                "--crate-type=rlib",
                "--crate-name=borrowed_operands_fixture",
            ]);
            if standard {
                compiler.args(["--cfg", "feature=\"std\""]);
            }
            if optimized {
                compiler.args(["-C", "opt-level=3", "-C", "overflow-checks=yes"]);
            }
            succeeds(
                compiler
                    .arg(scratch.0.join("src/lib.rs"))
                    .arg("-o")
                    .arg(&library),
            );
            let executable = scratch.0.join(format!("runner_{standard}_{optimized}"));
            succeeds(
                Command::new("rustc")
                    .arg("--edition=2021")
                    .arg(&runner)
                    .arg("--extern")
                    .arg(format!("borrowed_operands_fixture={}", library.display()))
                    .arg("-o")
                    .arg(&executable),
            );
            succeeds(&mut Command::new(&executable));
        }
    }
}

#[test]
fn read_only_owned_operands_fit_actual_wasm_memory_bound() {
    actual_wasm(
        "entry",
        1_048_576,
        1_048_576,
        80,
        "borrowed_operands_wasm_test.mjs",
    );
}

#[test]
fn owned_list_tail_executes_in_actual_bounded_wasm() {
    actual_wasm(
        "owned_list_entry",
        262_144,
        262_149,
        64,
        "owned_list_tail_wasm_test.mjs",
    );
}

#[test]
fn owned_record_field_executes_in_actual_bounded_wasm() {
    actual_wasm(
        "owned_field_entry",
        1_048_576,
        1_048_576,
        64,
        "borrowed_operands_wasm_test.mjs",
    );
}

#[test]
fn owned_match_result_executes_in_actual_bounded_wasm() {
    // Four one-MiB buffers (input ABI, Vec, owned result, output ABI), plus
    // stack/data. Verify the actual minimum, not a larger arbitrary ceiling.
    for pages in [66, 65] {
        actual_wasm(
            "owned_result_entry",
            1_048_576,
            1_048_576,
            pages,
            "owned_result_wasm_test.mjs",
        );
    }
}

#[test]
fn exclusive_owned_branches_execute_in_actual_bounded_wasm() {
    // Sixty-four transfers of the same one-MiB allocation must not copy it.
    // Input/output ABI copies remain subject to the existing fixture cap.
    actual_wasm(
        "branch_entry",
        1_048_576,
        1_048_576,
        64,
        "borrowed_operands_wasm_test.mjs",
    );
}

#[test]
fn borrowed_copy_patterns_execute_in_actual_bounded_wasm() {
    actual_wasm(
        "copy_pattern_entry",
        1_048_576,
        1_048_576,
        64,
        "borrowed_operands_wasm_test.mjs",
    );
}

#[test]
fn borrowed_utf8_decoding_executes_in_actual_bounded_wasm() {
    for release in [false, true] {
        actual_wasm_profile(
            "decode_field_entry",
            4096,
            4096,
            16,
            "borrowed_decode_wasm_test.mjs",
            release,
        );
    }
}

#[test]
fn leading_length_reads_move_within_actual_wasm_memory_bound() {
    for release in [false, true] {
        actual_wasm_profile(
            "length_entry",
            1_048_576,
            1_048_576,
            50,
            "borrowed_operands_wasm_test.mjs",
            release,
        );
    }
}

#[test]
fn branch_local_length_reads_move_within_actual_wasm_memory_bound() {
    for release in [false, true] {
        actual_wasm_profile(
            "branch_length_entry",
            1_048_576,
            1_048_576,
            50,
            "borrowed_operands_wasm_test.mjs",
            release,
        );
    }
}

#[test]
fn self_append_reuses_owned_storage_in_actual_bounded_wasm() {
    for release in [false, true] {
        actual_wasm_profile(
            "self_append",
            1_048_576,
            2_097_152,
            98,
            "self_append_wasm_test.mjs",
            release,
        );
    }
}

fn actual_wasm(entry: &str, input_cap: u32, output_cap: u32, pages: u32, script: &str) {
    actual_wasm_profile(entry, input_cap, output_cap, pages, script, true);
}

fn actual_wasm_profile(
    entry: &str,
    input_cap: u32,
    output_cap: u32,
    pages: u32,
    script: &str,
    release: bool,
) {
    let (remaining, module) = parse_module(IR).unwrap();
    assert!(remaining.is_empty());
    let package = generate_core_wasm_package(
        &module,
        &CoreWasmSpec {
            crate_name: "borrowed-operands-guest".into(),
            entry: entry.into(),
            export_name: "holo_run".into(),
            input_allocation_cap: input_cap,
            output_allocation_cap: output_cap,
            maximum_pages: pages,
            input_ir_sha256: format!("{:x}", Sha256::digest(IR.as_bytes())),
        },
    )
    .unwrap();
    let scratch = Scratch::new();
    for file in package.files {
        let path = scratch.0.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.bytes).unwrap();
    }
    let mut build = Command::new("cargo");
    if release {
        build.arg("build").arg("--release");
    } else {
        build.arg("build");
    }
    succeeds(
        build
            .current_dir(&scratch.0)
            .args(["--locked", "--offline"])
            .env_remove("RUSTC_WRAPPER")
            .env("CARGO_TARGET_DIR", scratch.0.join("target")),
    );
    succeeds(
        Command::new("node")
            .arg(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(script),
            )
            .arg(scratch.0.join(format!(
                "target/wasm32-unknown-unknown/{}/borrowed_operands_guest.wasm",
                if release { "release" } else { "debug" }
            )))
            .arg(pages.to_string()),
    );
}
