//! Compiles build metadata into the binary. Every value has a fallback: a tarball build has no git.
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

// Civil date from days since 1970-01-01 (Hinnant); avoids a date crate for one line of output.
fn ymd(secs: u64) -> String {
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn main() {
    // Same rule as packaging/macos/build-testflight.sh: GNOBBI_BUILD, else the commit count.
    let build = std::env::var("GNOBBI_BUILD")
        .ok()
        .filter(|b| !b.is_empty())
        .or_else(|| git(&["rev-list", "--count", "HEAD"]))
        .unwrap_or_else(|| "sconosciuto".into());
    // A tree without .git (update-mac.sh, the iOS bundle) gets the commit from GNOBBI_COMMIT, like the build number.
    let sha = std::env::var("GNOBBI_COMMIT")
        .ok()
        .filter(|c| !c.is_empty())
        .or_else(|| git(&["rev-parse", "--short", "HEAD"]))
        .unwrap_or_else(|| "sconosciuto".into());
    let secs = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.parse().ok())
        .or_else(|| {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
            t.ok().map(|d| d.as_secs())
        })
        .unwrap_or(0);
    let lock = std::fs::read_to_string("../../Cargo.lock").unwrap_or_default();
    let sdk = lock
        .split("[[package]]")
        .find(|p| p.contains("\nname = \"matrix-sdk\"\n"))
        .and_then(|p| p.lines().find_map(|l| l.strip_prefix("version = \"")))
        .map(|v| v.trim_end_matches('"').to_string())
        .unwrap_or_else(|| "sconosciuto".into());

    // Only a build made with an explicit GNOBBI_BUILD is a distributed one (updater on); a git commit count alone proves nothing.
    if std::env::var("GNOBBI_BUILD").is_ok_and(|b| !b.is_empty()) {
        println!("cargo:rustc-env=GNOBBI_DISTRIBUTED=1");
    }
    println!("cargo:rustc-env=GNOBBI_BUILD_NUMBER={build}");
    println!("cargo:rustc-env=GNOBBI_GIT_SHA={sha}");
    println!("cargo:rustc-env=GNOBBI_BUILD_DATE={}", ymd(secs));
    println!("cargo:rustc-env=GNOBBI_MATRIX_SDK_VERSION={sdk}");
    println!("cargo:rerun-if-env-changed=GNOBBI_BUILD");
    println!("cargo:rerun-if-env-changed=GNOBBI_COMMIT");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=../../Cargo.lock");
    if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={head}");
    }
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=gnobbi.rc");
        println!("cargo:rerun-if-changed=../../packaging/windows/gnobbi.ico");
        embed_resource::compile("gnobbi.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}
