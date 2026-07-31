use super::*;
use std::fs;

fn test_root() -> (tempfile::TempDir, DocumentResources) {
    let directory = tempfile::tempdir().expect("temporary resource root");
    fs::create_dir(directory.path().join("images")).expect("image directory");
    fs::write(directory.path().join("images/ok.png"), b"png").expect("image fixture");
    let resources = DocumentResources::new(ResourceAccess::Sanitized, Some(directory.path()), None);
    (directory, resources)
}

#[test]
fn sanitized_local_reference_requires_an_authorized_root() {
    let resources = DocumentResources::new(ResourceAccess::Sanitized, None, None);
    assert_eq!(resources.resolve("../../private.png", None), None);
    assert_eq!(
        resources.rewrite_css_urls("a{background:url(../../private.png)}", None),
        "a{background:url(\"\")}"
    );
}

#[test]
fn authorized_root_rewrites_descendants_and_rejects_traversal() {
    let (directory, resources) = test_root();
    let resolved = resources
        .resolve("images/ok.png", Some(directory.path()))
        .expect("authorized descendant");
    assert!(Path::new(&resolved).is_absolute());
    assert!(resolved.ends_with("images/ok.png"));
    assert_eq!(
        resources.resolve("../outside.png", Some(directory.path())),
        None
    );
}

#[test]
fn distinct_base_and_root_allow_shared_ancestor_assets_only() {
    let root = tempfile::tempdir().expect("authorized root");
    let base = root.path().join("cases/fonts");
    fs::create_dir_all(&base).expect("document base");
    fs::create_dir(root.path().join("fonts")).expect("shared font directory");
    fs::write(root.path().join("fonts/Parity.ttf"), b"font").expect("shared font");
    let outside = tempfile::tempdir().expect("outside directory");
    fs::write(outside.path().join("secret.ttf"), b"secret").expect("outside font");

    let resources =
        DocumentResources::new(ResourceAccess::Sanitized, Some(&base), Some(root.path()));
    let resolved = resources
        .resolve("../../fonts/Parity.ttf", resources.base_path())
        .expect("shared asset inside the explicit root");
    assert!(resolved.ends_with("fonts/Parity.ttf"));
    assert_eq!(
        resources.resolve(
            outside
                .path()
                .join("secret.ttf")
                .to_str()
                .expect("UTF-8 fixture path"),
            None
        ),
        None
    );
}

#[test]
fn base_outside_explicit_root_denies_relative_resources() {
    let root = tempfile::tempdir().expect("authorized root");
    let outside = tempfile::tempdir().expect("outside base");
    fs::write(outside.path().join("secret.png"), b"secret").expect("outside resource");
    let resources = DocumentResources::new(
        ResourceAccess::Sanitized,
        Some(outside.path()),
        Some(root.path()),
    );

    assert_eq!(resources.base_path(), None);
    assert_eq!(resources.resolve("secret.png", None), None);
}

#[cfg(unix)]
#[test]
fn authorized_root_rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let (directory, resources) = test_root();
    let outside = tempfile::tempdir().expect("outside directory");
    fs::write(outside.path().join("secret.png"), b"secret").expect("outside fixture");
    symlink(outside.path(), directory.path().join("linked")).expect("symlink fixture");

    assert_eq!(
        resources.resolve("linked/secret.png", Some(directory.path())),
        None
    );
}

#[test]
fn css_rewriter_ignores_comments_and_strings() {
    let resources = DocumentResources::new(ResourceAccess::Sanitized, None, None);
    let css =
        r#"/* url(secret.png) */ a::before{content:"url(secret.png)";background:url(secret.png)}"#;
    assert_eq!(
        resources.rewrite_css_urls(css, None),
        r#"/* url(secret.png) */ a::before{content:"url(secret.png)";background:url("")}"#
    );
}

#[test]
fn sanitized_css_preserves_inline_and_fragment_urls() {
    let resources = DocumentResources::new(ResourceAccess::Sanitized, None, None);
    let css = "a{filter:url(#fx);background:url(DATA:image/png;base64,AA==)}";
    assert_eq!(
        resources.rewrite_css_urls(css, None),
        "a{filter:url(\"#fx\");background:url(\"DATA:image/png;base64,AA==\")}"
    );
}

// A protocol-relative `//host` is denied in Trusted mode (no fetchable scheme)
// rather than read as a local file that escapes the root; a genuine traversal
// is still confined, showing the fix targets the misclassification, not all loads.
#[test]
fn trusted_protocol_relative_reference_cannot_escape_root() {
    let root = tempfile::tempdir().expect("authorized root");
    let resources = DocumentResources::new(ResourceAccess::Trusted, None, Some(root.path()));
    assert_eq!(resources.resolve("//etc/hostname", None), None);
    assert_eq!(resources.resolve("//../secret.png", None), None);
    assert_eq!(resources.resolve("../secret.png", None), None);
}

#[test]
fn trusted_network_urls_pass_case_insensitively() {
    let root = tempfile::tempdir().expect("authorized root");
    let resources = DocumentResources::new(ResourceAccess::Trusted, None, Some(root.path()));
    assert_eq!(
        resources.resolve("http://example.com/a.png", None).as_deref(),
        Some("http://example.com/a.png")
    );
    assert_eq!(
        resources.resolve("HTTPS://Example.com/a.png", None).as_deref(),
        Some("HTTPS://Example.com/a.png")
    );
}

#[test]
fn sanitized_denies_protocol_relative_reference() {
    let (directory, resources) = test_root();
    assert_eq!(
        resources.resolve("//etc/hostname", Some(directory.path())),
        None
    );
}
