use borrowed_utf8::{
    alias, borrowed, borrowedRecord, branch, copied, duplicate, entry, indexed, indexedLocal,
    nested, ownedRecord, single, sliced, slicedLocal, TextBox,
};

#[test]
fn owned_and_borrowed_payloads_preserve_values_and_single_use_allocations() {
    for text in ["", "ascii", "é\0𐐷", "\u{feff}edge"] {
        let expected = text.repeat(2).into_bytes();
        assert_eq!(duplicate(Some(text.into())), expected);
        assert_eq!(alias(Some(text.into())), expected);
        assert_eq!(nested(Some(Some(text.into()))), expected);
        assert_eq!(ownedRecord(text.into()), expected);
        assert_eq!(indexed(&[text.into()]), expected);
        assert_eq!(indexedLocal(&[text.into()]), expected);
        assert_eq!(sliced(text.as_bytes().to_vec()), expected);
        assert_eq!(slicedLocal(text.as_bytes().to_vec()), expected);
        assert_eq!(
            borrowed_utf8::indexedByte(text.as_bytes()),
            !text.is_empty()
        );
        let value = Some(text.to_owned());
        assert!(borrowed(value.clone()));
        assert_eq!(value.as_deref(), Some(text));
        let record = TextBox { text: text.into() };
        assert!(borrowedRecord(&record));
        assert_eq!(record.text, text);
        for flag in [false, true] {
            let mut owned = String::with_capacity(128);
            owned.push_str(text);
            let pointer = owned.as_ptr();
            let capacity = owned.capacity();
            let result = branch(Some(owned), flag);
            assert_eq!(result, text);
            assert_eq!(result.as_ptr(), pointer);
            assert_eq!(result.capacity(), capacity);
        }
        let mut owned = String::with_capacity(128);
        owned.push_str(text);
        let pointer = owned.as_ptr();
        let capacity = owned.capacity();
        let result = single(Some(owned));
        assert_eq!(result, text);
        assert_eq!(result.as_ptr(), pointer);
        assert_eq!(result.capacity(), capacity);
        assert_eq!(entry(text.as_bytes().to_vec()), text.as_bytes());
    }
    assert!(copied(Some(u64::MAX)));
    assert!(!copied(None));
    assert_eq!(duplicate(None), Vec::<u8>::new());
    assert_eq!(indexed(&[]), Vec::<u8>::new());
    assert_eq!(indexedLocal(&[]), Vec::<u8>::new());
    assert!(borrowed_utf8::indexedByte(&[255]));
    assert_eq!(nested(Some(None)), Vec::<u8>::new());
    assert_eq!(nested(None), Vec::<u8>::new());
    assert!(!borrowed(None));
    assert_eq!(entry(vec![255]), [255]);
}
