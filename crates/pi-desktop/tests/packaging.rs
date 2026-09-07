//! v0.5 packaging contract: local unsigned bundle, embedded sidecar, no updater.

use std::path::PathBuf;

#[test]
fn tauri_bundle_is_local_unsigned_with_sidecar() {
    let conf: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
    assert_eq!(conf["productName"], "pi");
    assert_eq!(conf["version"], "0.5.0");
    assert_eq!(conf["bundle"]["active"], true);
    assert_eq!(conf["bundle"]["createUpdaterArtifacts"], false);
    let bins = conf["bundle"]["externalBin"]
        .as_array()
        .expect("externalBin");
    assert!(
        bins.iter()
            .any(|b| b.as_str() == Some("binaries/pi-app-server")),
        "{bins:?}"
    );
    assert_eq!(conf["bundle"]["macOS"]["signingIdentity"], "-");
    assert!(conf["bundle"]["windows"]["certificateThumbprint"].is_null());
    let targets = conf["bundle"]["targets"].as_array().expect("targets");
    for wanted in ["app", "dmg", "nsis"] {
        assert!(
            targets.iter().any(|t| t.as_str() == Some(wanted)),
            "missing target {wanted}"
        );
    }
    assert!(
        !targets.iter().any(|t| t.as_str() == Some("appimage")),
        "Linux appimage is not a v0.5 product"
    );
    assert!(conf.get("plugins").is_none());
    let before = conf["build"]["beforeBuildCommand"].as_str().unwrap_or("");
    assert!(before.contains("prepare_sidecar.py"), "{before}");
}

#[test]
fn bundle_icons_are_checked_in() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("icons");
    for name in [
        "32x32.png",
        "128x128.png",
        "128x128@2x.png",
        "icon.icns",
        "icon.ico",
    ] {
        let path = dir.join(name);
        assert!(path.is_file(), "missing {path:?}");
        assert!(
            path.metadata().unwrap().len() > 32,
            "{name} looks empty"
        );
    }
}

#[test]
fn desktop_crate_does_not_depend_on_updater_or_shell_plugin() {
    let cargo = include_str!("../Cargo.toml");
    assert!(!cargo.contains("tauri-plugin-updater"));
    assert!(!cargo.contains("tauri-plugin-shell"));
}

#[test]
fn packaging_scripts_exist() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace")
        .to_path_buf();
    for rel in [
        "packaging/prepare_sidecar.py",
        "packaging/build-desktop.py",
        "packaging/README.md",
        ".github/workflows/release-desktop.yml",
        ".github/workflows/linux.yml",
    ] {
        assert!(root.join(rel).is_file(), "{rel}");
    }
}

#[test]
fn release_desktop_workflow_is_native_unsigned() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace")
        .to_path_buf();
    let yml = std::fs::read_to_string(root.join(".github/workflows/release-desktop.yml"))
        .expect("release-desktop.yml");
    for needle in [
        "macos-latest",
        "windows-latest",
        "packaging/build-desktop.py",
        "workflow_dispatch",
        "v*",
        "softprops/action-gh-release",
    ] {
        assert!(yml.contains(needle), "missing {needle}");
    }
    assert!(!yml.contains("tauri-plugin-updater"));
    assert!(!yml.contains("createUpdaterArtifacts: true"));
    assert!(!yml.contains("notarize"), "must not notarize");
    assert!(
        !yml.to_ascii_lowercase().contains("appimage"),
        "Linux AppImage is not a product"
    );
}
