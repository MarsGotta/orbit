use super::*;

#[test]
fn a_copy_for_opening_is_deleted_after_a_day() -> Result<()> {
    let root = tempfile::tempdir()?;
    let folder = root.path().join("attachments");
    let old = write_private(&folder.join("id-viejo"), "viejo.pdf", b"%PDF")?;
    let fresh = write_private(&folder.join("id-nuevo"), "nuevo.pdf", b"%PDF")?;
    let now = std::time::SystemTime::now();
    std::fs::File::open(folder.join("id-viejo"))?
        .set_modified(now - SAVED_ATTACHMENT_TTL - std::time::Duration::from_secs(60))?;
    retire_saved(&folder, now);
    assert!(!old.exists() && !folder.join("id-viejo").exists());
    assert!(fresh.exists());
    Ok(())
}

#[test]
fn a_saved_attachment_is_private_and_never_follows_a_link() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir()?;
    let directory = root.path().join("attachments").join("id-1");
    let path = write_private(&directory, "informe.pdf", b"%PDF-1.4")?;
    assert_eq!(std::fs::read(&path)?, b"%PDF-1.4");
    assert_eq!(
        std::fs::metadata(&path)?.permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(&directory)?.permissions().mode() & 0o777,
        0o700
    );
    // A name that would leave the folder is written under a plain one.
    let safe = write_private(&directory, "..", b"x")?;
    assert_eq!(safe, directory.join("adjunto"));
    // A link where the file goes is refused.
    std::os::unix::fs::symlink(root.path().join("fuera"), directory.join("enlace.txt"))?;
    assert!(write_private(&directory, "enlace.txt", b"x").is_err());
    assert!(!root.path().join("fuera").exists());
    Ok(())
}
