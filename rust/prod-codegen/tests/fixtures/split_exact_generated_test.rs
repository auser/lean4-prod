use typed_decimal::{splitBounded, splitEntry, splitMaximum, splitOne, splitZero};

#[test]
fn exact_field_limits_and_contents() {
    for (value, delimiter, expected) in [
        ("", "|", vec![""]),
        ("a", "|", vec!["a"]),
        ("a|b", "|", vec!["a", "b"]),
        ("|", "|", vec!["", ""]),
        ("a|b|c", "|", vec!["a", "b", "c"]),
        ("é|𐐷", "|", vec!["é", "𐐷"]),
        ("a\0|b", "|", vec!["a\0", "b"]),
        ("a::::b", "::", vec!["a", "", "b"]),
        ("aébé", "é", vec!["a", "b", ""]),
    ] {
        assert_eq!(
            splitMaximum(value.into(), delimiter.into()),
            Some(expected.iter().map(|s| s.to_string()).collect())
        );
        assert_eq!(
            splitBounded(value.into(), delimiter.into(), u32::MAX).unwrap(),
            expected
        );
        assert_eq!(splitZero(value.into(), delimiter.into()), None);
        assert_eq!(
            splitOne(value.into(), delimiter.into()).is_some(),
            expected.len() == 1
        );
        assert_eq!(
            splitBounded(value.into(), delimiter.into(), expected.len() as u32).unwrap(),
            expected
        );
        assert_eq!(
            splitBounded(value.into(), delimiter.into(), expected.len() as u32 - 1),
            None
        );
        assert_eq!(splitBounded(value.into(), "".into(), u32::MAX), None);
        assert_eq!(splitMaximum(value.into(), "".into()), None);
    }
    for (value, expected) in [
        ("", [1, 0, 1, 1]),
        ("a", [1, 0, 1, 1]),
        ("|", [1, 0, 0, 1]),
        ("a|b", [1, 0, 0, 1]),
        ("a|b|c", [1, 0, 0, 0]),
        ("é|𐐷", [1, 0, 0, 1]),
    ] {
        assert_eq!(splitEntry(value.as_bytes().to_vec()), expected);
    }
    assert_eq!(splitEntry(vec![255]), [255]);
}
