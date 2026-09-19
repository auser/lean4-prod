use borrowed_utf8::{
    aliasedLength, borrowedLength, entry, ownedEncode, recordLength, repeatedLength, TextBox,
};

#[test]
fn borrowed_and_owned_utf8_preserve_exact_bytes() {
    for text in ["", "ascii", "é\0𐐷", "\u{feff}edge", "a\0b"] {
        let expected = text.as_bytes();
        assert_eq!(borrowedLength(text.to_owned()), expected.len() as u64);
        assert_eq!(aliasedLength(text.to_owned()), expected.len() as u64);
        assert_eq!(
            repeatedLength(text.to_owned()).unwrap(),
            (expected.len() * 2) as u64
        );
        assert_eq!(
            recordLength(&TextBox {
                text: text.to_owned()
            }),
            expected.len() as u64
        );
        assert_eq!(ownedEncode(text.to_owned()), expected);
        assert_eq!(entry(expected.to_vec()).unwrap(), expected);
        assert_eq!(borrowedLength(text.to_owned()), expected.len() as u64);
    }
    assert_eq!(entry(vec![0xff]).unwrap(), vec![0xff]);
    let mut owned = String::with_capacity(128);
    owned.push_str("é\0𐐷");
    let pointer = owned.as_ptr();
    let capacity = owned.capacity();
    let encoded = ownedEncode(owned);
    assert_eq!(
        encoded.as_ptr(),
        pointer,
        "owned encoding must reuse its buffer"
    );
    assert_eq!(encoded.capacity(), capacity);
}
