#![cfg(unix)] // POSIX path literals / Linux desktop entry: is_absolute is host-specific.
use matrix_desktop::SignInOptions;
#[test]
fn graphical_signin_uses_explicit_private_profile_without_argv_credentials() {
    assert!(SignInOptions::parse(&[]).unwrap().is_none());
    let args = ["--signin", "/private/dime"].map(str::to_owned);
    let options = SignInOptions::parse(&args).unwrap().unwrap();
    assert_eq!(options.root.to_str(), Some("/private/dime"));
    assert!(options.ca_file.is_none());
    let args = ["--signin", "/private/dime", "--ca-file", "/private/ca.pem"].map(str::to_owned);
    assert!(
        SignInOptions::parse(&args)
            .unwrap()
            .unwrap()
            .ca_file
            .is_some()
    );
    for args in [
        vec!["--signin"],
        vec!["--signin", "relative"],
        vec!["--signin", "/private/dime", "password"],
        vec!["--signin", "/private/dime", "--ca-file", "relative"],
        vec!["--signin", "/private/dime", "--disable-tls", "/ca"],
    ] {
        assert!(
            SignInOptions::parse(&args.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err()
        );
    }
}
