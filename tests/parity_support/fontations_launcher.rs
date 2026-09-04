#[cfg(unix)]
#[test]
fn oracle_launcher_isolates_fontations_from_custom_fontconfig() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    let directory = tempfile::tempdir().expect("temporary launcher directory");
    let chromium = directory.path().join("chromium-probe");
    std::fs::write(
        &chromium,
        r#"#!/usr/bin/env bash
set -euo pipefail
test "${FONTCONFIG_FILE+x}" != x
test "${FONTCONFIG_PATH+x}" != x
test "${FC_FONTATIONS:-}" = 1
case " $* " in
  *" --enable-features=FontationsFontBackend,FontationsLinuxSystemFonts "*) ;;
  *) exit 1 ;;
esac
"#,
    )
    .expect("fake Chromium launcher");
    std::fs::set_permissions(&chromium, std::fs::Permissions::from_mode(0o755))
        .expect("executable fake Chromium launcher");

    let status = Command::new(format!(
        "{}/scripts/chromium-fontations.sh",
        env!("CARGO_MANIFEST_DIR")
    ))
    .arg("--headless=new")
    .env("IRONPRESS_CHROMIUM_EXECUTABLE", chromium)
    .env("FONTCONFIG_FILE", "/untrusted/fonts.conf")
    .env("FONTCONFIG_PATH", "/untrusted/fonts")
    .status()
    .expect("run Fontations launcher");

    assert!(status.success());
}
