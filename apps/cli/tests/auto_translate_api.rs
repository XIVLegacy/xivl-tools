use xivl_cli::auto_translate::{Locale, ResolveErrorKind, Resolver};

#[test]
fn reusable_context_handles_messages_without_loading_unused_resources() {
    let mut resolver = Resolver::new("not-loaded", Locale::En).unwrap();
    for _ in 0..2 {
        let (document, failed) = resolver.resolve_message(b"Authored message").unwrap();
        assert!(!failed);
        assert_eq!(document["text"], "Authored message");
        let (document, failed) = resolver.resolve_message(b"A\x02\x37\xff").unwrap();
        assert!(failed);
        assert_eq!(document["outputHex"], "410237ff");
    }
    let error = resolver.resolve([8, 974], None).unwrap_err();
    assert_eq!(error.kind(), ResolveErrorKind::Io);
    assert_eq!(error.code(), 1);
    assert!(!error.message().is_empty());
    assert_eq!(error.to_string(), error.message());
    assert_eq!(
        resolver
            .resolve([8, 974], Some([0, 1, 5, 0]))
            .unwrap_err()
            .kind(),
        ResolveErrorKind::Io
    );
}

#[test]
fn context_refuses_wrong_locale_for_explicit_english_noun_inputs() {
    let mut resolver = Resolver::new("not-loaded", Locale::Ja).unwrap();
    let error = resolver
        .resolve([21, 1_000_001], Some([0, 1, 5, 0]))
        .unwrap_err();
    assert_eq!(error.kind(), ResolveErrorKind::Usage);
}
