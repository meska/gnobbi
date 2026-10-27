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

#[cfg(unix)]
#[test]
fn bundle_profile_directory_is_created_private() {
    use std::os::unix::fs::PermissionsExt;
    let base = std::env::temp_dir().join(format!("dime-profile-test-{}", std::process::id()));
    let profile = base.join("Library/Application Support/Dime/profile");
    matrix_desktop::create_private_dir(&profile).unwrap();
    for dir in [&profile, &base.join("Library/Application Support/Dime")] {
        assert_eq!(
            std::fs::metadata(dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    matrix_desktop::create_private_dir(&profile).unwrap(); // idempotent
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn bare_signin_uses_the_profile_under_the_data_directory() {
    use matrix_desktop::{AppDirs, default_signin_args};
    let dirs = AppDirs {
        config: "/c/dime".into(),
        data: "/d/dime".into(),
        cache: "/k/dime".into(),
    };
    let bare = vec!["--signin".to_owned()];
    assert_eq!(
        default_signin_args(&bare, &dirs),
        vec!["--signin".to_owned(), "/d/dime/profile".to_owned()]
    );
    let with_ca = ["--signin", "--ca-file", "/ca.pem"].map(str::to_owned);
    assert_eq!(
        default_signin_args(&with_ca, &dirs),
        ["--signin", "/d/dime/profile", "--ca-file", "/ca.pem"].map(str::to_owned)
    );
    let given = vec!["--signin".to_owned(), "/p".to_owned()];
    assert_eq!(default_signin_args(&given, &dirs), given);
    assert!(default_signin_args(&[], &dirs).is_empty());
}

#[test]
fn linux_launch_without_arguments_opens_the_sign_in_on_the_standard_profile() {
    use matrix_desktop::{AppDirs, default_signin_args, linux_launch_args};
    let dirs = AppDirs {
        config: "/c/dime".into(),
        data: "/d/dime".into(),
        cache: "/k/dime".into(),
    };
    let args = linux_launch_args(&[]);
    assert_eq!(args, ["--signin"]);
    assert_eq!(
        default_signin_args(&args, &dirs),
        ["--signin", "/d/dime/profile"]
    );
    // Explicit flags keep working for tests.
    let given = ["--signin", "/p"].map(str::to_owned);
    assert_eq!(linux_launch_args(&given), given);
    let restore = ["--restore", "/s", "1"].map(str::to_owned);
    assert_eq!(linux_launch_args(&restore), restore);
}
