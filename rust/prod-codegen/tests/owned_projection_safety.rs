//! Execute ownership-sensitive projections through the public raw-IR boundary.

use prod_codegen::{generate_cargo_package, CargoPackageSpec};
use prod_ir::parser::parse_module;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::Command;

const IR: &str = r#"(module OwnedProjectionSafety
  (type "EmptyRecord" (ctor "EmptyRecord.mk"))
  (type "EmptyVariant" (ctor "EmptyVariant.first") (ctor "EmptyVariant.second"))
  (type "Parcel" (ctor "Parcel.mk" (bytes Bytes) (marker Nat)))
  (type "Envelope" (ctor "Envelope.mk" (parcel (named "Parcel")) (label String)))
  (type "MaybeEnvelope" (ctor "MaybeEnvelope.mk" (parcel (Option (named "Parcel")))))
  (type "Pair" (ctor "Pair.mk" (first Bytes) (second Bytes)))
  (type "OptionalPair" (ctor "OptionalPair.mk" (first (Option Bytes)) (second (Option Bytes))))
  (type "ParcelList" (ctor "ParcelList.mk" (slots (List (named "Parcel")))))
  (type "VariantBox" (ctor "VariantBox.mk" (value (Option (named "EmptyVariant"))) (bytes Bytes)))
  (def equal_variant ((left (named "EmptyVariant")) (right (named "EmptyVariant"))) Bool
    (eq left right))
  (def borrowed_result_error ((input (Result (named "Parcel") (named "EmptyVariant"))) (expected (named "EmptyVariant"))) Bool
    (cases input
      (alt "Except.ok" (value) false)
      (alt "Except.error" (failure) (call equal_variant failure expected))))
  (def borrowed_result_ok ((input (Result (named "EmptyVariant") (named "Parcel"))) (expected (named "EmptyVariant"))) Bool
    (let alias input
      (cases alias
        (alt "Except.ok" (value) (call equal_variant value expected))
        (alt "Except.error" (failure) false))))
  (def borrowed_option_variant ((input (named "VariantBox")) (expected (named "EmptyVariant"))) Bool
    (let value (proj "VariantBox" "value" input)
      (cases value
        (alt "Option.none" () false)
        (alt "Option.some" (actual) (call equal_variant actual expected)))))
  (def borrowed_nested_variant ((input (Option (Result (named "EmptyVariant") (named "Parcel")))) (expected (named "EmptyVariant"))) Bool
    (cases input
      (alt "Option.none" () false)
      (alt "Option.some" (result)
        (cases result
          (alt "Except.ok" (actual) (call equal_variant actual expected))
          (alt "Except.error" (failure) false)))))
  (def owned_option_variant ((input (Option (named "EmptyVariant")))) (named "EmptyVariant")
    (cases input
      (alt "Option.none" () (ctor "EmptyVariant.first"))
      (alt "Option.some" (actual) actual)))
  (def owned_result_variant ((input (Result (named "EmptyVariant") (named "Parcel")))) String
    (cases input
      (alt "Except.ok" (actual)
        (if (call equal_variant actual (ctor "EmptyVariant.first")) (string "first") (string "second")))
      (alt "Except.error" (failure) (string "error"))))
  (def borrowed_copy_record ((input (Result (named "EmptyRecord") (named "Parcel")))) Nat
    (cases input
      (alt "Except.ok" (actual) (call match_empty_record actual))
      (alt "Except.error" (failure) 0)))
  (def borrowed_copy_nat ((input (Result Nat (named "Parcel")))) Nat
    (cases input
      (alt "Except.ok" (actual) (call counted_nat actual))
      (alt "Except.error" (failure) 0)))
  (def counted_nat ((input Nat)) Nat (ctor "counted_value" input))
  (def choose_label ((left (named "Envelope")) (right (named "Envelope")) (flag Bool)) String
    (cases flag
      (alt "Bool.false" () (let label (proj "Envelope" "label" left) label))
      (alt "Bool.true" () (let label (proj "Envelope" "label" right) label))))
  (def choose_bytes ((left (named "Parcel")) (right (named "Parcel")) (flag Bool)) Bytes
    (if flag (proj "Parcel" "bytes" right) (proj "Parcel" "bytes" left)))
  (def choose_record ((left (named "Envelope")) (right (named "Envelope")) (flag Bool)) (named "Parcel")
    (cases flag
      (alt "Bool.false" () (let parcel (proj "Envelope" "parcel" left) parcel))
      (alt "Bool.true" () (let parcel (proj "Envelope" "parcel" right) parcel))))
  (def choose_option ((left (named "MaybeEnvelope")) (right (named "MaybeEnvelope")) (flag Bool)) (Option (named "Parcel"))
    (if flag (proj "MaybeEnvelope" "parcel" right) (proj "MaybeEnvelope" "parcel" left)))
  (def choose_list_owner ((left (named "ParcelList")) (right (named "ParcelList")) (flag Bool)) (named "ParcelList")
    (ctor "ParcelList.mk"
      (cases flag
        (alt "Bool.false" () (let slots (proj "ParcelList" "slots" left) slots))
        (alt "Bool.true" () (let slots (proj "ParcelList" "slots" right) slots)))))
  (def local_owner_label ((input String)) String
    (let owner (ctor "Envelope.mk" (ctor "Parcel.mk" (bytes 9) 7) input)
      (let first (proj "Envelope" "label" owner)
        (let second (proj "Envelope" "label" owner) (if (eq first second) first second)))))
  (def fallible_label ((left (named "Envelope")) (right (named "Envelope")) (flag Bool) (input Nat)) String
    (let checked (add input 1)
      (let counted (ctor "counted_value" checked)
        (cases flag
          (alt "Bool.false" () (let label (proj "Envelope" "label" left) label))
          (alt "Bool.true" () (let label (proj "Envelope" "label" right) label))))))
  (def borrowed_label_accessor ((owner (named "Envelope"))) String
    (let label (proj "Envelope" "label" owner) label))
  (def borrowed_positional_accessor ((owner (named "Envelope"))) String
    (let label (proj "Envelope" "label" (param 0)) label))
  (def ambiguous_label_accessor ((owner (named "Envelope")) (other (named "Envelope"))) String
    (let label (proj "Envelope" "label" owner) label))
  (def temporary_label () String
    (let label (proj "Envelope" "label" (ctor "Envelope.mk" (ctor "Parcel.mk" (bytes) 0) (string "temporary"))) label))
  (def local_nested_return ((input (Option (named "Envelope"))) (flag Bool)) (Option (named "ParcelList"))
    (cases input
      (alt "Option.none" () (ctor "Option.none"))
      (alt "Option.some" (owner)
        (ctor "Option.some" (ctor "ParcelList.mk"
          (let local (ctor "ParcelList.mk" (ctor "List.cons" (proj "Envelope" "parcel" owner) (ctor "List.nil")))
            (let first (proj "ParcelList" "slots" local)
              (let second (proj "ParcelList" "slots" local) (if flag first second)))))))))
  (def large_nat_add () Nat (let high 4294967295 (add high 1)))
  (def large_nat_mul () Nat (let high 4294967296 (mul high 2)))
  (def large_nat_sub () Nat (let high 18446744073709551615 (let alias high (sub alias 1))))
  (def large_nat_div () Nat (let high 18446744073709551615 (div high 3)))
  (def large_nat_mod () Nat (let high 18446744073709551615 (mod high 3)))
  (def large_nat_shl () Nat (let high 4294967296 (shl high 1)))
  (def large_nat_shr () Nat (let high 18446744073709551615 (shr high 32)))
  (def large_nat_pow () Nat (let high 4294967296 (pow high 1)))
  (def large_nat_add_overflow () Nat (let high 18446744073709551615 (add high 1)))
  (def large_nat_mul_overflow () Nat (let high 18446744073709551615 (mul high 2)))
  (def large_nat_divisor () Nat (let high 18446744073709551615 (div 4294967295 high)))
  (def large_nat_remainder () Nat (let high 18446744073709551615 (mod 4294967295 high)))
  (def large_nat_shl_exponent () Nat (let high 18446744073709551615 (shl 1 high)))
  (def large_nat_pow_exponent () Nat (let high 18446744073709551615 (pow 1 high)))
  (def large_nat_shr_exponent () Nat (let high 18446744073709551615 (shr 1 high)))
  (def contextual_uint8 () UInt8 (let value 255 value))
  (def contextual_uint32 () UInt32 (let value 4294967295 value))
  (def contextual_int32 () Int32 (let value 2147483647 value))
  (def contextual_int64 () Int64 (let value 9223372036854775807 value))
  (def empty_record () (named "EmptyRecord") (ctor "EmptyRecord.mk"))
  (def match_empty_record ((input (named "EmptyRecord"))) Nat
    (cases input (alt "EmptyRecord.mk" () 17)))
  (def empty_variant () (named "EmptyVariant") (ctor "EmptyVariant.first"))
  (def match_empty_variant ((input (named "EmptyVariant"))) Nat
    (cases input (alt "EmptyVariant.first" () 19) (alt "EmptyVariant.second" () 23)))
  (def empty_order () Ordering (compare-bytes (bytes) (bytes)))
  (def aliased_empty_order () Ordering
    (let empty (bytes) (compare-bytes empty empty)))
  (def literal_order () Ordering (compare-bytes (bytes 0 255) (bytes 128)))
  (def empty_left ((input Bytes)) Ordering (compare-bytes (bytes) input))
  (def empty_right ((input Bytes)) Ordering (compare-bytes input (bytes)))
  (def join_failure_order ((input Nat)) Nat
    (let continuation (jp continuation (first second) 0)
      (jmp continuation (add input 1) (mul input 2))))
  (def panic_value () Nat (unreachable))
  (def join_panic () Nat
    (let continuation (jp continuation (unused) 7)
      (jmp continuation (unreachable))))
  (def join_called_panic () Nat
    (let continuation (jp continuation (unused) 7)
      (jmp continuation (call panic_value))))
  (def join_panic_before_error ((input Nat)) Nat
    (let continuation (jp continuation (first second) 7)
      (jmp continuation (unreachable) (add input 1))))
  (def join_error_before_panic ((input Nat)) Nat
    (let continuation (jp continuation (first second) 7)
      (jmp continuation (add input 1) (unreachable))))
  (def unused_let_panic () Nat (let unused (call panic_value) 7))
  (def join_conditional_panic ((fail Bool)) Nat
    (let continuation (jp continuation (unused) 7)
      (jmp continuation (if fail (unreachable) 0))))
  (def join_once ((input Nat)) Nat
    (let continuation (jp continuation (value) (add value value))
      (jmp continuation (ctor "counted_value" input))))
  (def join_unused_order () Nat
    (let continuation (jp continuation (first second) 7)
      (jmp continuation (ctor "counted_value" 1) (ctor "counted_value" 2))))
  (def join_partial_failure ((input Nat)) Nat
    (let continuation (jp continuation (first second third) 7)
      (jmp continuation (ctor "counted_value" 1) (add input 1) (ctor "counted_value" 2))))
  (def non_tail_calls ((capture Nat)) Nat
    (let function (jp function (value) (add capture value))
      (let first (jmp function 2)
        (let second (jmp function 3) (add first second)))))
  (def non_tail_capture_shadow ((capture Nat)) Nat
    (let function (jp function (value) (add capture value))
      (let capture 100 (let result (jmp function 2) (add result capture)))))
  (def non_tail_unused_failure ((input Nat)) Nat
    (let function (jp function (unused) 7)
      (let ignored (jmp function (add input 1)) 19)))
  (def non_tail_order () Nat
    (let function (jp function (value) (ctor "counted_value" value))
      (let first (jmp function 1)
        (let second (jmp function 2) (ctor "counted_value" 3)))))
  (def borrowed_head ((input (List (named "Parcel")))) (Option (named "Parcel"))
    (cases input
      (alt "List.nil" () (ctor "Option.none"))
      (alt "List.cons" (head tail) (ctor "Option.some" head))))
  (def borrowed_rebuild ((input (List (named "Parcel")))) (named "ParcelList")
    (cases input
      (alt "List.nil" () (ctor "ParcelList.mk" (ctor "List.nil")))
      (alt "List.cons" (head tail)
        (ctor "ParcelList.mk" (ctor "List.cons" head tail)))))
  (def borrowed_prepend ((head (named "Parcel")) (tail (List (named "Parcel")))) (named "ParcelList")
    (ctor "ParcelList.mk" (ctor "List.cons" head tail)))
  (def borrowed_projected_tail ((head (named "Parcel")) (tail (named "ParcelList"))) (named "ParcelList")
    (let slots (proj "ParcelList" "slots" tail)
      (ctor "ParcelList.mk" (ctor "List.cons" head slots))))
  (def owned_list ((head (Option (named "Parcel"))) (tail (Option (List (named "Parcel"))))) (Option (named "ParcelList"))
    (cases head
      (alt "Option.none" () (ctor "Option.none"))
      (alt "Option.some" (item)
        (cases tail
          (alt "Option.none" () (ctor "Option.none"))
          (alt "Option.some" (items)
            (ctor "Option.some" (ctor "ParcelList.mk" (ctor "List.cons" item items))))))))
  (def owned_singleton ((input (Option (named "Parcel")))) (Option (named "ParcelList"))
    (cases input
      (alt "Option.none" () (ctor "Option.none"))
      (alt "Option.some" (head)
        (ctor "Option.some" (ctor "ParcelList.mk" (ctor "List.cons" head (ctor "List.nil")))))))
  (def mixed_join ((input (List (named "Parcel"))) (replacement Bytes) (choose Bool)) (Option (named "ParcelList"))
    (cases input
      (alt "List.nil" () (ctor "Option.none"))
      (alt "List.cons" (head tail)
        (let continuation
          (jp continuation (item)
            (let alias item
              (ctor "Option.some" (ctor "ParcelList.mk" (ctor "List.cons" alias tail)))))
          (if choose
            (jmp continuation (ctor "Parcel.mk" replacement 42))
            (jmp continuation head))))))
  (def nested_join ((input (named "Parcel"))) (Option (named "Parcel"))
    (let outer
      (jp outer (item)
        (let alias item
          (let inner (jp inner (value) (ctor "Option.some" value))
            (jmp inner alias))))
      (jmp outer input)))
  (def nested_owned ((input (Option (named "Envelope")))) (Option (named "Pair"))
    (cases input
      (alt "Option.none" () (ctor "Option.none"))
      (alt "Option.some" (outer)
        (let inner (proj "Envelope" "parcel" outer)
          (let label (proj "Envelope" "label" outer)
            (let bytes (proj "Parcel" "bytes" inner)
              (let marker (proj "Parcel" "marker" inner)
                (if (eq marker 11)
                  (ctor "Option.some" (ctor "Pair.mk" bytes (utf8-encode label)))
                  (ctor "Option.none")))))))))
  (def borrowed_optional ((input (named "MaybeEnvelope"))) (Option Bytes)
    (let optional (proj "MaybeEnvelope" "parcel" input)
      (cases optional
        (alt "Option.none" () (ctor "Option.none"))
        (alt "Option.some" (inner)
          (ctor "Option.some" (proj "Parcel" "bytes" inner))))))
  (def borrowed_named ((input (named "Envelope"))) (named "Pair")
    (let inner (proj "Envelope" "parcel" input)
      (let label (proj "Envelope" "label" input)
        (ctor "Pair.mk" (proj "Parcel" "bytes" inner) (utf8-encode label)))))
  (def mixed_match ((input (Option (named "Parcel"))) (fallback (named "Parcel"))) (Option Bytes)
    (let selected
      (cases input
        (alt "Option.none" () (proj "Parcel" "bytes" fallback))
        (alt "Option.some" (inner) (proj "Parcel" "bytes" inner)))
      (ctor "Option.some" selected)))
  (def captured_join ((input Bytes)) (named "OptionalPair")
    (let owner (ctor "Parcel.mk" input 11)
      (let continuation
        (jp continuation () (ctor "Option.some" (proj "Parcel" "bytes" owner)))
        (let first (jmp continuation)
          (let second (jmp continuation)
            (ctor "OptionalPair.mk" first second))))))
  (def borrowed_join ((input (named "Parcel"))) (Option Bytes)
    (let continuation
      (jp continuation (row) (ctor "Option.some" (proj "Parcel" "bytes" row)))
      (jmp continuation input)))
  (def positional_shadow ((row (named "Parcel")) (input (Option (named "Parcel")))) (Option (named "Pair"))
    (cases input
      (alt "Option.none" () (ctor "Option.none"))
      (alt "Option.some" (row)
        (let original (proj "Parcel" "bytes" (param 0))
          (let selected (proj "Parcel" "bytes" row)
            (ctor "Option.some" (ctor "Pair.mk" original selected)))))))
  (def nested_shadow ((input (Option (named "Envelope")))) (Option Bytes)
    (cases input
      (alt "Option.none" () (ctor "Option.none"))
      (alt "Option.some" (row)
        (let row (proj "Envelope" "parcel" row)
          (let row (proj "Parcel" "bytes" row)
            (ctor "Option.some" row)))))))"#;

const RUNNER: &str = r#"use owned_projection_safety_fixture::*;

fn parcel(bytes: Vec<u8>, marker: u64) -> Parcel {
    Parcel { bytes, marker }
}

fn main() {
    let mut cases = 0;
    for size in [0, 1, 128, 8192] {
        let expected: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();
        assert_eq!(empty_left(expected.clone()), [].as_slice().cmp(expected.as_slice()));
        assert_eq!(empty_right(expected.clone()), expected.as_slice().cmp(&[]));
        cases += 2;
        let slots = vec![parcel(expected.clone(), 11), parcel(vec![254], 12)];
        let mut output = borrowed_head(&slots).unwrap();
        assert_eq!(output, slots[0]);
        output.bytes.push(255);
        assert_eq!(slots[0].bytes, expected);

        let mut output = borrowed_rebuild(&slots);
        assert_eq!(output.slots, slots);
        output.slots[0].bytes.push(255);
        output.slots[1].bytes.push(255);
        assert_eq!(slots[0].bytes, expected);
        assert_eq!(slots[1].bytes, [254]);

        let head = parcel(vec![253], 13);
        let mut output = nested_join(&head).unwrap();
        assert_eq!(output, head);
        output.bytes.push(255);
        assert_eq!(head.bytes, [253]);
        for choose in [false, true] {
            let replacement = vec![251; size + 1];
            let pointer = replacement.as_ptr();
            let mut output = mixed_join(&slots, replacement, choose).unwrap();
            assert_eq!(output.slots.len(), slots.len());
            if choose {
                assert_eq!(output.slots[0].bytes, vec![251; size + 1]);
                assert_eq!(output.slots[0].marker, 42);
                assert_eq!(output.slots[0].bytes.as_ptr(), pointer, "owned join argument moves");
            } else {
                assert_eq!(output.slots[0], slots[0]);
            }
            output.slots[0].bytes.push(255);
            assert_eq!(slots[0].bytes, expected);
            assert_eq!(output.slots[1], slots[1]);
        }
        cases += 3;
        let mut output = borrowed_prepend(&head, &slots);
        assert_eq!(output.slots, [vec![head.clone()], slots.clone()].concat());
        output.slots[0].bytes.push(255);
        output.slots[1].bytes.push(255);
        assert_eq!(head.bytes, [253]);
        assert_eq!(slots[0].bytes, expected);

        let list = ParcelList { slots };
        let mut output = borrowed_projected_tail(&head, &list);
        assert_eq!(output.slots, [vec![head.clone()], list.slots.clone()].concat());
        output.slots[2].bytes.push(255);
        assert_eq!(list.slots[1].bytes, [254]);

        let mut tail = Vec::with_capacity(4);
        tail.push(parcel(vec![252], 14));
        let list_pointer = tail.as_ptr();
        let tail_pointer = tail[0].bytes.as_ptr();
        let head = parcel(expected.clone(), 11);
        let head_pointer = head.bytes.as_ptr();
        let output = owned_list(Some(head), Some(tail)).unwrap();
        assert_eq!(output.slots.len(), 2);
        assert_eq!(output.slots[0].bytes, expected);
        assert_eq!(output.slots[1].bytes, [252]);
        assert_eq!(output.slots.as_ptr(), list_pointer, "owned list capacity is reused");
        assert_eq!(output.slots[1].bytes.as_ptr(), tail_pointer, "owned tail item moves");
        if size != 0 {
            assert_eq!(output.slots[0].bytes.as_ptr(), head_pointer, "owned head moves");
        }

        let head = parcel(expected.clone(), 11);
        let head_pointer = head.bytes.as_ptr();
        let output = owned_singleton(Some(head)).unwrap();
        assert_eq!(output.slots.len(), 1);
        assert_eq!(output.slots[0].bytes, expected);
        if size != 0 {
            assert_eq!(output.slots[0].bytes.as_ptr(), head_pointer, "singleton moves its item");
        }
        cases += 6;
        for label in ["", "parcel", "🌱\0é"] {
            let bytes = expected.clone();
            let pointer = bytes.as_ptr();
            let label_value = label.to_owned();
            let label_pointer = label_value.as_ptr();
            let output = nested_owned(Some(Envelope {
                parcel: parcel(bytes, 11), label: label_value,
            })).unwrap();
            assert_eq!(output.first, expected);
            assert_eq!(output.second, label.as_bytes());
            if !expected.is_empty() {
                assert_eq!(output.first.as_ptr(), pointer, "nested field keeps its allocation");
            }
            if !label.is_empty() {
                assert_eq!(output.second.as_ptr(), label_pointer, "disjoint String field moves too");
            }
            assert!(nested_owned(Some(Envelope {
                parcel: parcel(expected.clone(), 12), label: label.to_owned(),
            })).is_none());

            let borrowed = Envelope {
                parcel: parcel(expected.clone(), 11), label: label.to_owned(),
            };
            let mut output = borrowed_named(&borrowed);
            assert_eq!(output.first, expected);
            assert_eq!(output.second, label.as_bytes());
            output.first.push(255);
            output.second.push(255);
            assert_eq!(borrowed.parcel.bytes, expected);
            assert_eq!(borrowed.label, label);
            assert_eq!(borrowed_named(&borrowed).first, expected);

            let bytes = expected.clone();
            let pointer = bytes.as_ptr();
            let output = nested_shadow(Some(Envelope {
                parcel: parcel(bytes, 11), label: label.to_owned(),
            })).unwrap();
            assert_eq!(output, expected);
            if !expected.is_empty() {
                assert_eq!(output.as_ptr(), pointer, "normalized inner binders retain ownership");
            }
            cases += 4;
        }

        let borrowed = MaybeEnvelope { parcel: Some(parcel(expected.clone(), 11)) };
        let mut output = borrowed_optional(&borrowed).unwrap();
        assert_eq!(output, expected);
        output.push(255);
        assert_eq!(borrowed.parcel.as_ref().unwrap().bytes, expected);
        assert_eq!(borrowed_optional(&borrowed).unwrap(), expected);

        let fallback_bytes = vec![254; size + 1];
        let fallback = parcel(fallback_bytes.clone(), 11);
        let bytes = expected.clone();
        let pointer = bytes.as_ptr();
        let output = mixed_match(Some(parcel(bytes, 11)), &fallback).unwrap();
        assert_eq!(output, expected);
        if !expected.is_empty() {
            assert_eq!(output.as_ptr(), pointer, "owned Match arm moves its field");
        }
        let mut output = mixed_match(None, &fallback).unwrap();
        assert_eq!(output, fallback_bytes);
        output.push(255);
        assert_eq!(fallback.bytes, fallback_bytes);

        let mut output = captured_join(expected.clone());
        assert_eq!(output.first.as_deref(), Some(expected.as_slice()));
        assert_eq!(output.second.as_deref(), Some(expected.as_slice()));
        output.first.as_mut().unwrap().push(255);
        assert_eq!(output.second.as_deref(), Some(expected.as_slice()));

        let borrowed = parcel(expected.clone(), 11);
        let mut output = borrowed_join(&borrowed).unwrap();
        assert_eq!(output, expected);
        output.push(255);
        assert_eq!(borrowed.bytes, expected);

        let bytes = expected.clone();
        let pointer = bytes.as_ptr();
        let mut output = positional_shadow(&fallback, Some(parcel(bytes, 11))).unwrap();
        assert_eq!(output.first, fallback_bytes);
        assert_eq!(output.second, expected);
        if !expected.is_empty() {
            assert_eq!(output.second.as_ptr(), pointer, "shadowing does not borrow the owned payload");
        }
        output.first.push(255);
        assert_eq!(fallback.bytes, fallback_bytes);
        assert!(positional_shadow(&fallback, None).is_none());
        cases += 7;
    }
    assert!(nested_owned(None).is_none());
    assert!(nested_shadow(None).is_none());
    assert!(borrowed_optional(&MaybeEnvelope { parcel: None }).is_none());
    cases += 3;
    assert!(borrowed_head(&[]).is_none());
    assert!(borrowed_rebuild(&[]).slots.is_empty());
    assert!(owned_list(None, None).is_none());
    assert!(owned_singleton(None).is_none());
    cases += 4;
    assert_eq!(empty_order(), core::cmp::Ordering::Equal);
    assert_eq!(aliased_empty_order(), core::cmp::Ordering::Equal);
    assert_eq!(literal_order(), core::cmp::Ordering::Less);
    cases += 3;
    assert!(mixed_join(&[], vec![1], true).is_none());
    cases += 1;
    assert_eq!(join_failure_order(0), Ok(0));
    assert_eq!(join_failure_order(u64::MAX), Err(ComputeError::AddOverflow));
    cases += 2;
    assert!(std::panic::catch_unwind(join_panic).is_err());
    assert!(std::panic::catch_unwind(join_called_panic).is_err());
    assert!(std::panic::catch_unwind(|| join_panic_before_error(u64::MAX)).is_err());
    assert_eq!(join_error_before_panic(u64::MAX), Err(ComputeError::AddOverflow));
    assert!(std::panic::catch_unwind(|| join_error_before_panic(0)).is_err());
    assert!(std::panic::catch_unwind(unused_let_panic).is_err());
    assert_eq!(join_conditional_panic(false), 7);
    assert!(std::panic::catch_unwind(|| join_conditional_panic(true)).is_err());
    cases += 8;
    reset_evaluation_probe();
    assert_eq!(join_once(5), Ok(10));
    assert_eq!((evaluation_calls(), evaluation_order()), (1, 5));
    reset_evaluation_probe();
    assert_eq!(join_unused_order(), 7);
    assert_eq!((evaluation_calls(), evaluation_order()), (2, 12));
    reset_evaluation_probe();
    assert_eq!(join_partial_failure(u64::MAX), Err(ComputeError::AddOverflow));
    assert_eq!((evaluation_calls(), evaluation_order()), (1, 1));
    reset_evaluation_probe();
    assert_eq!(join_partial_failure(0), Ok(7));
    assert_eq!((evaluation_calls(), evaluation_order()), (2, 12));
    cases += 4;
    assert_eq!(non_tail_calls(5), Ok(15));
    assert_eq!(non_tail_capture_shadow(5), Ok(107));
    assert_eq!(non_tail_unused_failure(u64::MAX), Err(ComputeError::AddOverflow));
    assert_eq!(non_tail_unused_failure(0), Ok(19));
    reset_evaluation_probe();
    assert_eq!(non_tail_order(), 3);
    assert_eq!((evaluation_calls(), evaluation_order()), (3, 123));
    cases += 5;
    assert_eq!(match_empty_record(empty_record()), 17);
    assert_eq!(match_empty_variant(empty_variant()), 19);
    assert_eq!(match_empty_variant(EmptyVariant::second), 23);
    cases += 3;
    assert_eq!(large_nat_add(), Ok(4294967296));
    assert_eq!(large_nat_mul(), Ok(8589934592));
    assert_eq!(large_nat_sub(), u64::MAX - 1);
    assert_eq!(large_nat_div(), u64::MAX / 3);
    assert_eq!(large_nat_mod(), 0);
    assert_eq!(large_nat_shl(), Ok(8589934592));
    assert_eq!(large_nat_shr(), 4294967295);
    assert_eq!(large_nat_pow(), Ok(4294967296));
    assert_eq!(large_nat_add_overflow(), Err(ComputeError::AddOverflow));
    assert_eq!(large_nat_mul_overflow(), Err(ComputeError::MulOverflow));
    assert_eq!(large_nat_divisor(), 0);
    assert_eq!(large_nat_remainder(), 4294967295);
    assert_eq!(large_nat_shl_exponent(), Err(ComputeError::ShiftExponentTooLarge));
    assert_eq!(large_nat_pow_exponent(), Err(ComputeError::PowExponentTooLarge));
    assert_eq!(large_nat_shr_exponent(), 0);
    assert_eq!(contextual_uint8(), u8::MAX);
    assert_eq!(contextual_uint32(), u32::MAX);
    assert_eq!(contextual_int32(), i32::MAX);
    assert_eq!(contextual_int64(), i64::MAX);
    cases += 19;
    let left = Envelope { parcel: Parcel { bytes: vec![], marker: 11 }, label: "left".into() };
    let right = Envelope { parcel: Parcel { bytes: vec![1, 2, 3], marker: 17 }, label: "right".into() };
    let no_parcel = MaybeEnvelope { parcel: None };
    let some_parcel = MaybeEnvelope { parcel: Some(right.parcel.clone()) };
    let left_list = ParcelList { slots: vec![left.parcel.clone(), right.parcel.clone()] };
    let right_list = ParcelList { slots: vec![right.parcel.clone()] };
    for flag in [false, true] {
        let expected = if flag { &right } else { &left };
        assert_eq!(choose_label(&left, &right, flag), expected.label);
        assert_eq!(choose_bytes(&left.parcel, &right.parcel, flag), expected.parcel.bytes);
        assert_eq!(choose_record(&left, &right, flag), expected.parcel);
        assert_eq!(choose_option(&no_parcel, &some_parcel, flag), if flag { some_parcel.parcel.clone() } else { None });
        assert_eq!(choose_list_owner(&left_list, &right_list, flag), if flag { right_list.clone() } else { left_list.clone() });
        reset_evaluation_probe();
        assert_eq!(fallible_label(&left, &right, flag, 0), Ok(expected.label.clone()));
        assert_eq!((evaluation_calls(), evaluation_order()), (1, 1));
        cases += 6;
    }
    assert_eq!(local_owner_label("local".into()), "local");
    reset_evaluation_probe();
    assert_eq!(fallible_label(&left, &right, false, u64::MAX), Err(ComputeError::AddOverflow));
    assert_eq!((evaluation_calls(), evaluation_order()), (0, 0));
    assert!(std::ptr::eq(borrowed_label_accessor(&left), &left.label));
    cases += 3;
    for flag in [false, true] {
        assert_eq!(local_nested_return(Some(left.clone()), flag), Some(ParcelList { slots: vec![left.parcel.clone()] }));
        cases += 1;
    }
    assert_eq!(local_nested_return(None, false), None);
    cases += 1;
    assert!(std::ptr::eq(borrowed_positional_accessor(&left), &left.label));
    assert_eq!(ambiguous_label_accessor(&left, &right), left.label);
    assert_eq!(temporary_label(), "temporary");
    cases += 3;
    for expected in [EmptyVariant::first, EmptyVariant::second] {
        for actual in [EmptyVariant::first, EmptyVariant::second] {
            assert_eq!(borrowed_result_error(Err(actual), expected), actual == expected);
            assert_eq!(borrowed_result_ok(Ok(actual), expected), actual == expected);
            assert_eq!(borrowed_option_variant(&VariantBox { value: Some(actual), bytes: vec![1] }, expected), actual == expected);
            assert_eq!(borrowed_nested_variant(Some(Ok(actual)), expected), actual == expected);
            cases += 4;
        }
        assert!(!borrowed_result_error(Ok(parcel(vec![1], 2)), expected));
        assert!(!borrowed_result_ok(Err(parcel(vec![1], 2)), expected));
        assert!(!borrowed_option_variant(&VariantBox { value: None, bytes: vec![1] }, expected));
        assert!(!borrowed_nested_variant(Some(Err(parcel(vec![1], 2))), expected));
        assert!(!borrowed_nested_variant(None, expected));
        cases += 5;
    }
    assert_eq!(owned_option_variant(None), EmptyVariant::first);
    assert_eq!(owned_option_variant(Some(EmptyVariant::first)), EmptyVariant::first);
    assert_eq!(owned_option_variant(Some(EmptyVariant::second)), EmptyVariant::second);
    assert_eq!(owned_result_variant(Ok(EmptyVariant::first)), "first");
    assert_eq!(owned_result_variant(Ok(EmptyVariant::second)), "second");
    assert_eq!(owned_result_variant(Err(parcel(vec![1], 2))), "error");
    assert_eq!(borrowed_copy_record(Ok(EmptyRecord {})), 17);
    assert_eq!(borrowed_copy_record(Err(parcel(vec![1], 2))), 0);
    reset_evaluation_probe();
    assert_eq!(borrowed_copy_nat(Ok(42)), 42);
    assert_eq!((evaluation_calls(), evaluation_order()), (1, 42));
    assert_eq!(borrowed_copy_nat(Err(parcel(vec![1], 2))), 0);
    assert_eq!((evaluation_calls(), evaluation_order()), (1, 42));
    cases += 12;
    assert_eq!(cases, 231);
    println!("owned projection safety: {cases} cases passed");
}
"#;

// Test-only instrumentation for the compiler's documented bare host-constructor
// boundary. It observes evaluation count/order without changing generated IR
// or supplying application behavior, and also compiles in a no_std library.
const EVALUATION_PROBE: &str = r#"
static EVALUATION_CALLS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
static EVALUATION_ORDER: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
pub fn reset_evaluation_probe() {
    EVALUATION_CALLS.store(0, core::sync::atomic::Ordering::SeqCst);
    EVALUATION_ORDER.store(0, core::sync::atomic::Ordering::SeqCst);
}
pub fn evaluation_calls() -> usize { EVALUATION_CALLS.load(core::sync::atomic::Ordering::SeqCst) }
pub fn evaluation_order() -> usize { EVALUATION_ORDER.load(core::sync::atomic::Ordering::SeqCst) }
fn counted_value(value: u64) -> u64 {
    EVALUATION_CALLS.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
    EVALUATION_ORDER.store(evaluation_order() * 10 + value as usize, core::sync::atomic::Ordering::SeqCst);
    value
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
            "prod-owned-projection-safety-{}-{nonce}",
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

fn succeeds(command: &mut Command) -> String {
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn owned_projection_safety_executes_in_std_and_no_std_debug_and_optimized() {
    let (remaining, module) = parse_module(IR).unwrap();
    assert!(remaining.is_empty(), "the complete fixture must parse");
    let spec = CargoPackageSpec {
        name: "owned-projection-safety-fixture".into(),
        version: "0.1.0".into(),
        description: "Owned projection safety regression".into(),
        repository: "https://github.com/auser/lean4-prod".into(),
        homepage: "https://github.com/auser/lean4-prod".into(),
        readme: "Owned projection safety regression fixture.\n".into(),
        license_mit: "MIT\n".into(),
        license_apache: "Apache-2.0\n".into(),
        input_sha256: format!("{:x}", Sha256::digest(IR.as_bytes())),
        dependencies: vec![],
    };
    let package = generate_cargo_package(&module, &spec).unwrap();
    assert_eq!(package, generate_cargo_package(&module, &spec).unwrap());
    let scratch = Scratch::new();
    for mut file in package.files {
        if file.path == "src/lib.rs" {
            file.bytes.extend_from_slice(EVALUATION_PROBE.as_bytes());
        }
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
                "--crate-name=owned_projection_safety_fixture",
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
            let mut runner_compiler = Command::new("rustc");
            runner_compiler.arg("--edition=2021");
            if optimized {
                runner_compiler.args(["-C", "opt-level=3", "-C", "overflow-checks=yes"]);
            }
            succeeds(
                runner_compiler
                    .arg(&runner)
                    .arg("--extern")
                    .arg(format!(
                        "owned_projection_safety_fixture={}",
                        library.display()
                    ))
                    .arg("-o")
                    .arg(&executable),
            );
            assert_eq!(
                succeeds(&mut Command::new(&executable)),
                "owned projection safety: 231 cases passed\n"
            );
        }
    }
}
