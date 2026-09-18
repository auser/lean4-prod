use borrowed_utf8::{
    aliasBox, appendMatches, boxBytes, copyHeadAndTail, entry, firstBox, firstBytes, nestedBytes,
    prepend, sameBytes, sameBytesReversed, sameStrings, ByteBox, TextBox, TextList,
};

#[test]
fn borrowed_collections_preserve_owners_and_exact_values() {
    for text in ["", "ascii", "é\0𐐷", "\u{feff}edge"] {
        let values = vec![text.to_owned()];
        let first = firstBox(&values);
        assert_eq!(first.text, text);
        assert_eq!(aliasBox(&values), first);
        assert!(appendMatches(text.as_bytes()));
        assert_eq!(firstBytes(&values), text.as_bytes());
        assert_eq!(boxBytes(&first), text.as_bytes());
        let list = TextList {
            values: values.clone(),
        };
        let extended = prepend(&list);
        assert_eq!(extended.values, ["prefix", text]);
        assert_eq!(nestedBytes(&extended.values), text.as_bytes());
        assert_eq!(list.values, values);
        let longer = vec![text.to_owned(), "tail".to_owned(), "last".to_owned()];
        assert_eq!(copyHeadAndTail(&longer).values, longer);
        assert_eq!(longer, [text, "tail", "last"]);
        let bytes = ByteBox {
            bytes: text.as_bytes().to_vec(),
        };
        assert!(sameBytes(text.as_bytes(), &bytes));
        assert!(sameBytesReversed(text.as_bytes(), &bytes));
        assert!(!sameBytes(&[0xff], &bytes));
        assert!(!sameBytesReversed(&[0xff], &bytes));
        assert!(sameStrings(text, &first));
        assert!(!sameStrings(
            "other",
            &TextBox {
                text: text.to_owned()
            }
        ));
        assert_eq!(entry(text.as_bytes().to_vec()), text.as_bytes());
        assert_eq!(values, [text]);
    }
    assert_eq!(firstBytes(&[]), Vec::<u8>::new());
    assert!(copyHeadAndTail(&[]).values.is_empty());
    assert_eq!(nestedBytes(&["one".to_owned()]), Vec::<u8>::new());
    assert_eq!(entry(vec![0xff]), vec![0xff]);
}
