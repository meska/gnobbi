use std::{fs, process::Command};

/// Compile real consumer snippets, checking exact error codes as well as a
/// positive control. Missing imports or toolchain failures cannot pass this test.
#[test]
fn compiler_enforces_identity_kinds_and_required_scope() {
    let deps = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    // A shared target dir holds one rlib per profile hash (dev build vs test of another
    // crate); the one this invocation just built is the newest.
    let library = fs::read_dir(&deps)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("libmatrix_domain-")
                && path
                    .extension()
                    .is_some_and(|extension| extension == "rlib")
        })
        .max_by_key(|path| fs::metadata(path).unwrap().modified().unwrap())
        .expect("a current domain library");
    let directory = deps.join(format!("identity-contract-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let cases = [
        (
            "valid",
            "let scope = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new(\"!room:example.invalid\") }; let _ = ScopedTimelineId { scope: scope.clone(), id: TimelineId::Pending(TransactionId::new(\"t\")) }; let _ = ScopedTimelineId { scope, id: TimelineId::Server(EventId::new(\"$e\")) };",
            None,
        ),
        (
            "event_as_pending",
            "let _ = TimelineId::Pending(EventId::new(\"same\"));",
            Some("E0308"),
        ),
        (
            "transaction_as_server",
            "let _ = TimelineId::Server(TransactionId::new(\"same\"));",
            Some("E0308"),
        ),
        (
            "room_as_event",
            "let _ = TimelineId::Server(RoomId::new(\"same\"));",
            Some("E0308"),
        ),
        (
            "missing_scope",
            "let _ = ScopedTimelineId { id: TimelineId::Pending(TransactionId::new(\"t\")) };",
            Some("E0063"),
        ),
        (
            "missing_account",
            "let _ = RoomScope { room_id: RoomId::new(\"!room:example.invalid\") };",
            Some("E0063"),
        ),
        (
            "missing_room",
            "let _ = RoomScope { account_id: AccountId::new(1) };",
            Some("E0063"),
        ),
    ];
    let result = std::panic::catch_unwind(|| {
        for (name, body, expected_error) in cases {
            let source = directory.join(format!("{name}.rs"));
            fs::write(&source, format!("use matrix_domain::{{AccountId,RoomId,EventId,TransactionId,RoomScope,TimelineId,ScopedTimelineId}}; fn main() {{ {body} }}")).unwrap();
            let output = Command::new("rustc")
                .args([
                    "--edition=2024",
                    "--emit=metadata",
                    "--error-format=json",
                    "--extern",
                ])
                .arg(format!("matrix_domain={}", library.display()))
                .arg("-L")
                .arg(format!("dependency={}", deps.display()))
                .arg(&source)
                .arg("-o")
                .arg(directory.join(format!("{name}.rmeta")))
                .output()
                .unwrap();
            let diagnostics = String::from_utf8(output.stderr).unwrap();
            match expected_error {
                None => assert!(output.status.success(), "{name}: {diagnostics}"),
                Some(code) => {
                    assert!(!output.status.success(), "{name} unexpectedly compiled");
                    assert!(
                        diagnostics.contains(&format!("\"code\":\"{code}\"")),
                        "{name}: {diagnostics}"
                    );
                    assert!(
                        !diagnostics.contains("\"code\":\"E0432\""),
                        "missing API: {diagnostics}"
                    );
                }
            }
        }
    });
    fs::remove_dir_all(directory).unwrap();
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}
