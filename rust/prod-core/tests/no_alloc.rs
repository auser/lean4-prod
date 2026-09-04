#![allow(non_snake_case)] // generated code mirrors Lean definition names

//! Certification that the generated code performs zero heap activity.
//!
//! `prod-core` has no `extern crate alloc`, so an allocating *type* could not
//! compile — but that says nothing about allocations hidden inside a call.
//! This test installs a counting global allocator and asserts the counter does
//! not move across the generated functions, including the list builder that
//! writes into a caller-owned buffer.
//!
//! Must run serially (`just no-alloc` passes `--test-threads=1`): the counter
//! is process-global.

use prod_alloc_counter::{activity, measure, CountingAllocator};
use prod_core::{
    belt, classDecode, classIndex, class_count, digitCount, digitSum, digits, sameClass,
    smallEnough, stride, tryClassDecode, ComputeError, Instance,
};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const CANONICAL: Instance = Instance { q: 4, T: 3, O: 8 };

/// Run `body`, and fail if it caused any heap activity.
///
/// The closure is `#[inline(never)]`-free on purpose: whatever the optimiser
/// does, an allocation would still pass through the global allocator.
fn assert_no_allocation<T>(what: &str, body: impl FnOnce() -> T) -> T {
    let (value, operations) = measure(body);
    assert_eq!(
        operations, 0,
        "{what} performed {} heap operation(s); generated code must be allocation-free",
        operations
    );
    value
}

/// One test, not several: the counter is process-global, so a sibling test
/// running concurrently would perturb it. Keeping the whole certification in a
/// single function makes it correct under any `--test-threads` setting.
#[test]
fn generated_definitions_never_touch_the_heap() -> Result<(), ComputeError> {
    // Guard against the certification passing vacuously: a mis-wired
    // `#[global_allocator]` would make every count zero.
    let before = activity();
    let boxed = Box::new(7u64);
    assert_eq!(*boxed, 7);
    drop(boxed);
    assert!(
        activity() > before,
        "the counting allocator is not installed; the no-alloc assertions prove nothing"
    );
    let (_, measured) = measure(|| drop(Box::new(7u64)));
    assert!(
        measured >= 2,
        "the thread-scoped allocation counter is not armed"
    );

    // Scalar definitions.
    assert_eq!(assert_no_allocation("stride", || stride(CANONICAL))?, 24);
    assert_eq!(
        assert_no_allocation("class_count", || class_count(CANONICAL))?,
        96
    );
    assert_eq!(assert_no_allocation("belt", || belt(CANONICAL))?, 12_288);
    assert_eq!(
        assert_no_allocation("classIndex", || classIndex(1, 2, 3, CANONICAL))?,
        43
    );
    assert_eq!(
        assert_no_allocation("classDecode", || classDecode(43, CANONICAL))?,
        (1, (2, 3))
    );

    // Recursion, guards, and `Option`.
    assert_eq!(
        assert_no_allocation("digitCount", || digitCount(10, 43, CANONICAL))?,
        2
    );
    assert!(assert_no_allocation("sameClass", || sameClass(
        43, 44, CANONICAL
    ))?);
    assert!(assert_no_allocation("smallEnough", || smallEnough(
        100, CANONICAL
    ))?);
    assert_eq!(
        assert_no_allocation("tryClassDecode", || tryClassDecode(43, CANONICAL))?,
        Some((1, (2, 3)))
    );

    // The list path: a caller-owned buffer in, a borrowed slice back out.
    // This is the case that used to allocate a `Box`-linked list per cons.
    let mut buffer = [0u64; 64];
    let len = assert_no_allocation("digits", || digits(10, 43, CANONICAL, &mut buffer))?;
    assert_eq!(&buffer[..len], &[3, 5]);
    assert_eq!(
        assert_no_allocation("digitSum", || digitSum(&buffer[..len]))?,
        8
    );

    // Error construction and propagation must be allocation-free too — an
    // error path that allocates is still an allocation on hostile input.
    let overflowing = Instance { q: 1, T: 1, O: 70 };
    assert_eq!(
        assert_no_allocation("belt overflow", || belt(overflowing)),
        Err(ComputeError::PowOverflow)
    );
    let mut too_small = [0u64; 1];
    assert_eq!(
        assert_no_allocation("digits into an undersized buffer", || digits(
            10,
            43,
            CANONICAL,
            &mut too_small
        )),
        Err(ComputeError::OutputTooSmall)
    );

    Ok(())
}
