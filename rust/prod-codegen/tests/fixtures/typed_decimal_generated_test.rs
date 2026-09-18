use typed_decimal::*;

#[test]
fn exact_width_and_canonical_decimal_boundaries() {
    macro_rules! check_unsigned {
        ($parse:ident, $max:expr, $overflow:expr) => {
            assert_eq!($parse("0".into()), Some(0));
            assert_eq!($parse($max.to_string()), Some($max));
            assert_eq!($parse($overflow.into()), None);
            for invalid in [
                "", "00", "01", "+1", "-1", "-0", " 1", "1 ", "1\n", "1\0", "１", "١",
            ] {
                assert_eq!(
                    $parse(invalid.into()),
                    None,
                    "{}: {invalid:?}",
                    stringify!($parse)
                );
            }
        };
    }
    macro_rules! check_signed {
        ($parse:ident, $min:expr, $max:expr, $underflow:expr, $overflow:expr) => {
            assert_eq!($parse("0".into()), Some(0));
            assert_eq!($parse("-1".into()), Some(-1));
            assert_eq!($parse($min.to_string()), Some($min));
            assert_eq!($parse($max.to_string()), Some($max));
            assert_eq!($parse($underflow.into()), None);
            assert_eq!($parse($overflow.into()), None);
            for invalid in [
                "", "00", "01", "+1", "-0", "-01", " 1", "1 ", "1\n", "1\0", "１", "١",
            ] {
                assert_eq!(
                    $parse(invalid.into()),
                    None,
                    "{}: {invalid:?}",
                    stringify!($parse)
                );
            }
        };
    }
    check_unsigned!(parseUint8, u8::MAX, "256");
    check_unsigned!(parseUint16, u16::MAX, "65536");
    check_unsigned!(parseUint32, u32::MAX, "4294967296");
    check_unsigned!(parseUint64, u64::MAX, "18446744073709551616");
    check_signed!(parseInt8, i8::MIN, i8::MAX, "-129", "128");
    check_signed!(parseInt16, i16::MIN, i16::MAX, "-32769", "32768");
    check_signed!(parseInt32, i32::MIN, i32::MAX, "-2147483649", "2147483648");
    check_signed!(
        parseInt64,
        i64::MIN,
        i64::MAX,
        "-9223372036854775809",
        "9223372036854775808"
    );
    for (text, expected) in [
        ("0", [1, 0]),
        ("1", [1, 1]),
        ("255", [1, 1]),
        ("256", [0, 1]),
        ("65535", [0, 1]),
        ("65536", [0, 0]),
        ("01", [0, 0]),
        ("+1", [0, 0]),
        ("-1", [0, 0]),
    ] {
        assert_eq!(acceptUInt8(text.into()), expected[0] == 1);
        assert_eq!(nonzeroUInt16(text.into()), expected[1] == 1);
        assert_eq!(entry(text.as_bytes().to_vec()), expected);
    }
    assert_eq!(entry(vec![0xff]), [255]);
}
