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

#[test]
fn bundle_launch_without_arguments_opens_connected_sign_in_in_the_app_profile() {
    use matrix_desktop::{SignInOptions, bundle_launch_args};
    let exe = "/Applications/Dime.app/Contents/MacOS/Dime";
    let home = Some("/Users/x/Library/Containers/net.meskatech.dime/Data");
    let args = bundle_launch_args(exe, home, &[]).unwrap();
    assert_eq!(
        args,
        [
            "--signin",
            "/Users/x/Library/Containers/net.meskatech.dime/Data/Library/Application Support/Dime/profile"
        ]
    );
    assert!(SignInOptions::parse(&args).unwrap().is_some());
    // Explicit arguments always win; a bare binary (tests, demo) stays offline.
    let given = vec!["--signin".to_owned(), "/p".to_owned()];
    assert!(bundle_launch_args(exe, home, &given).is_none());
    assert!(bundle_launch_args("/work/target/debug/matrix-desktop", home, &[]).is_none());
    assert!(bundle_launch_args(exe, None, &[]).is_none());
    assert!(bundle_launch_args(exe, Some("relative"), &[]).is_none());
}
