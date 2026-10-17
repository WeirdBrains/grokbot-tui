//! Default data directory (`~/.grokbot-tui`) and the one-time move from the pre-1.2
//! location (`~/.gb`). User data is never deleted: when the old folder can't simply be
//! renamed it is copied, verified and left in place.
use anyhow::{Context, Result, bail};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Default data directory name under the home directory.
pub const DIR_NAME: &str = ".grokbot-tui";
/// Data directory used before 1.2.0.
pub const LEGACY_DIR_NAME: &str = ".gb";
/// Marker inside the new directory: the old folder was already handled (copied or reported
/// as ignored), so later starts don't repeat the notice.
const MARKER: &str = ".migrated-from-gb";

/// The data directory to open and an optional notice for the user.
#[derive(Debug)]
pub struct Resolved {
    pub dir: PathBuf,
    pub notice: Option<String>,
}

/// Resolve the default data directory under `home`, migrating `~/.gb` if needed.
pub fn resolve(home: &Path) -> Resolved {
    migrate(&home.join(LEGACY_DIR_NAME), &home.join(DIR_NAME), |a, b| {
        fs::rename(a, b)
    })
}

/// Migration logic with an injectable `rename` (tests simulate a cross-filesystem failure).
/// - only `old` exists: rename it to `new`; if that fails, copy + verify, keep `old`.
/// - both exist: use `new`; report once that `old` was ignored.
/// - otherwise: use `new` (created later by `Store::open`).
pub fn migrate(
    old: &Path,
    new: &Path,
    rename: impl Fn(&Path, &Path) -> io::Result<()>,
) -> Resolved {
    let (o, n) = (old.display(), new.display());
    let new_exists = fs::symlink_metadata(new).is_ok();
    let old_is_dir = old.is_dir();
    if new_exists {
        let notice = (old_is_dir && !new.join(MARKER).exists()).then(|| {
            let _ = mark(new);
            format!(
                "Using {n}. The old data folder {o} was ignored and left untouched; \
                 delete it yourself once you no longer need it."
            )
        });
        return Resolved {
            dir: new.to_path_buf(),
            notice,
        };
    }
    if !old_is_dir {
        return Resolved {
            dir: new.to_path_buf(),
            notice: None,
        };
    }
    if rename(old, new).is_ok() {
        return Resolved {
            dir: new.to_path_buf(),
            notice: Some(format!("Moved the data folder from {o} to {n}.")),
        };
    }
    match copy_verified(old, new) {
        Ok(()) => {
            let _ = mark(new);
            Resolved {
                dir: new.to_path_buf(),
                notice: Some(format!(
                    "Copied the data folder from {o} to {n} (it could not be moved). \
                     The old folder was left in place; delete it yourself once you've checked {n}."
                )),
            }
        }
        Err(e) => Resolved {
            dir: old.to_path_buf(),
            notice: Some(format!(
                "Could not move or copy {o} to {n} ({e:#}); still using {o}."
            )),
        },
    }
}

fn mark(new: &Path) -> io::Result<()> {
    fs::write(new.join(MARKER), b"")
}

/// Copy `old` into a temporary sibling of `new`, verify every file, then rename it into
/// place. On failure only the temporary copy is removed; `old` is never modified.
fn copy_verified(old: &Path, new: &Path) -> Result<()> {
    let name = new.file_name().context("data dir has no name")?;
    let tmp = new.with_file_name(format!(
        "{}.partial-{}",
        name.to_string_lossy(),
        std::process::id()
    ));
    if fs::symlink_metadata(&tmp).is_ok() {
        bail!("{} already exists", tmp.display());
    }
    // From here on `tmp` is ours, so it may be removed if anything fails.
    let res = (|| {
        copy_tree(old, &tmp)?;
        verify_tree(old, &tmp)?;
        fs::rename(&tmp, new).with_context(|| format!("renaming into {}", new.display()))
    })();
    if res.is_err() {
        let _ = fs::remove_dir_all(&tmp);
    }
    res
}

fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir(dst).with_context(|| format!("creating {}", dst.display()))?;
    fs::set_permissions(dst, fs::metadata(src)?.permissions())?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let (from, to) = (entry.path(), dst.join(entry.file_name()));
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_tree(&from, &to)?;
        } else if ty.is_symlink() {
            copy_symlink(&from, &to)?;
        } else {
            // fs::copy also copies the permission bits (context.db stays 0600).
            fs::copy(&from, &to).with_context(|| format!("copying {}", from.display()))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(from: &Path, to: &Path) -> Result<()> {
    std::os::unix::fs::symlink(fs::read_link(from)?, to)?;
    Ok(())
}

#[cfg(not(unix))]
fn copy_symlink(from: &Path, to: &Path) -> Result<()> {
    fs::copy(from, to)?;
    Ok(())
}

/// Every regular file under `src` exists under `dst` with identical bytes.
fn verify_tree(src: &Path, dst: &Path) -> Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let (a, b) = (entry.path(), dst.join(entry.file_name()));
        let ty = entry.file_type()?;
        if ty.is_dir() {
            verify_tree(&a, &b)?;
        } else if ty.is_file() && fs::read(&a)? != fs::read(&b)? {
            bail!("verification failed for {}", a.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(dir: &Path) {
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("context.db"), b"sqlite bytes").unwrap();
        fs::write(dir.join("sub/note.txt"), b"hello").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(dir.join("context.db"), fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    fn paths(d: &tempfile::TempDir) -> (PathBuf, PathBuf) {
        (d.path().join(".gb"), d.path().join(".grokbot-tui"))
    }

    fn cross_device(_: &Path, _: &Path) -> io::Result<()> {
        Err(io::Error::other("cross-device link"))
    }

    #[test]
    fn resolve_uses_names_under_home() {
        let d = tempfile::tempdir().unwrap();
        let r = resolve(d.path());
        assert_eq!(r.dir, d.path().join(".grokbot-tui"));
        assert!(r.notice.is_none());
    }

    #[test]
    fn fresh_install_uses_new_dir_without_notice() {
        let d = tempfile::tempdir().unwrap();
        let (old, new) = paths(&d);
        let r = migrate(&old, &new, |a, b| fs::rename(a, b));
        assert_eq!(r.dir, new);
        assert!(r.notice.is_none());
        assert!(!old.exists());
    }

    #[test]
    fn only_old_dir_is_renamed() {
        let d = tempfile::tempdir().unwrap();
        let (old, new) = paths(&d);
        seed(&old);
        let r = migrate(&old, &new, |a, b| fs::rename(a, b));
        assert_eq!(r.dir, new);
        assert!(r.notice.unwrap().contains("Moved"));
        assert!(!old.exists());
        assert_eq!(fs::read(new.join("context.db")).unwrap(), b"sqlite bytes");
        assert_eq!(fs::read(new.join("sub/note.txt")).unwrap(), b"hello");
    }

    #[test]
    fn failed_rename_copies_verifies_and_keeps_old_dir() {
        let d = tempfile::tempdir().unwrap();
        let (old, new) = paths(&d);
        seed(&old);
        let r = migrate(&old, &new, cross_device);
        assert_eq!(r.dir, new);
        let notice = r.notice.unwrap();
        assert!(notice.contains("Copied") && notice.contains("left in place"));
        // Old data untouched, new copy identical.
        assert_eq!(fs::read(old.join("context.db")).unwrap(), b"sqlite bytes");
        assert_eq!(fs::read(new.join("context.db")).unwrap(), b"sqlite bytes");
        assert_eq!(fs::read(new.join("sub/note.txt")).unwrap(), b"hello");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&new), 0o700);
            assert_eq!(mode(&new.join("context.db")), 0o600);
        }
        // No temporary copy left behind.
        let leftovers: Vec<_> = fs::read_dir(d.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("partial"))
            .collect();
        assert!(leftovers.is_empty());
        // Next start: both exist, but the copy notice already covered it.
        let again = migrate(&old, &new, cross_device);
        assert_eq!(again.dir, new);
        assert!(again.notice.is_none());
    }

    #[test]
    fn both_dirs_use_new_and_notice_only_once() {
        let d = tempfile::tempdir().unwrap();
        let (old, new) = paths(&d);
        seed(&old);
        fs::create_dir_all(&new).unwrap();
        fs::write(new.join("context.db"), b"new db").unwrap();
        let first = migrate(&old, &new, |a, b| fs::rename(a, b));
        assert_eq!(first.dir, new);
        assert!(first.notice.unwrap().contains("ignored"));
        let second = migrate(&old, &new, |a, b| fs::rename(a, b));
        assert!(second.notice.is_none());
        // Neither folder's data was changed.
        assert_eq!(fs::read(old.join("context.db")).unwrap(), b"sqlite bytes");
        assert_eq!(fs::read(new.join("context.db")).unwrap(), b"new db");
    }

    #[test]
    fn only_new_dir_is_used_silently() {
        let d = tempfile::tempdir().unwrap();
        let (old, new) = paths(&d);
        fs::create_dir_all(&new).unwrap();
        let r = migrate(&old, &new, |a, b| fs::rename(a, b));
        assert_eq!(r.dir, new);
        assert!(r.notice.is_none());
        assert!(!new.join(MARKER).exists());
    }

    #[test]
    fn failed_copy_keeps_using_old_dir() {
        let d = tempfile::tempdir().unwrap();
        let (old, new) = paths(&d);
        seed(&old);
        // A stale temporary copy blocks the copy; the old folder must stay in use.
        let tmp = d
            .path()
            .join(format!(".grokbot-tui.partial-{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        let r = migrate(&old, &new, cross_device);
        assert_eq!(r.dir, old);
        assert!(r.notice.unwrap().contains("still using"));
        assert!(!new.exists());
        assert!(tmp.exists(), "a pre-existing folder is never removed");
        assert_eq!(fs::read(old.join("context.db")).unwrap(), b"sqlite bytes");
    }
}
