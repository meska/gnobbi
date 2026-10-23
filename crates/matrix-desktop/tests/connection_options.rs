use matrix_desktop::RestoreOptions;

#[test]
fn restore_arguments_are_explicit_and_never_contain_passphrases() {
    assert!(RestoreOptions::parse(&[]).unwrap().is_none());
    let args = [
        "--restore",
        "/private/store",
        "18",
        "--ca-file",
        "/private/ca.pem",
    ]
    .map(str::to_owned);
    let options = RestoreOptions::parse(&args).unwrap().unwrap();
    assert_eq!(options.account_id.as_u64(), 18);
    assert_eq!(options.root.to_str(), Some("/private/store"));
    assert_eq!(options.ca_file.unwrap().to_str(), Some("/private/ca.pem"));
    for invalid in [
        vec!["--password", "secret"],
        vec!["--restore"],
        vec!["--restore", "relative", "18"],
        vec!["--restore", "/store", "bad"],
        vec!["--restore", "/store", "18", "--disable-tls"],
        vec!["--restore", "/store", "18", "--ca-file", "relative"],
    ] {
        assert!(
            RestoreOptions::parse(&invalid.into_iter().map(str::to_owned).collect::<Vec<_>>())
                .is_err()
        );
    }
}
