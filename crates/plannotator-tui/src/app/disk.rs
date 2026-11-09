//! Noticing that the reviewed file changed on disk, typically an agent editing it, and
//! reloading it at once: the notes are found again in the new text, a note whose words are gone
//! is orphaned. While the person types, chooses or answers a question, the reload waits for
//! them to be done, the footer saying so; `reload` stays for a reload by hand.

use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use plannotator_tui_schema::Provenance;

use super::{App, Mode};

/// How often the event loop may stat the open file.
const CHECK_EVERY: Duration = Duration::from_secs(1);

/// What a file looked like on disk: enough to tell that it was rewritten without reading it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DiskStamp {
    modified: SystemTime,
    len: u64,
}

impl DiskStamp {
    /// The file's stamp now, or `None` when it is missing or cannot be read.
    pub(super) fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self { modified: meta.modified().ok()?, len: meta.len() })
    }

    /// The stamp of a document's file, when the document is one.
    pub(super) fn of_source(provenance: &Provenance) -> Option<Self> {
        match provenance {
            Provenance::File { path } => Self::of(path),
            _ => None,
        }
    }
}

impl App {
    /// Called from the event loop on every wake; checks at most once per `CHECK_EVERY`.
    /// Returns whether the screen needs a redraw.
    pub(crate) fn poll_disk(&mut self) -> bool {
        let now = Instant::now();
        if self.disk_checked.is_some_and(|last| now.duration_since(last) < CHECK_EVERY) {
            return false;
        }
        self.disk_checked = Some(now);
        self.check_disk()
    }

    /// Compare the open file with the stamp it had when it was loaded and reload it when it
    /// changed. While something is under way (typing, a selection waiting for its kind, a menu
    /// or a question) the reload waits, said once per change, and happens on a later check. A
    /// missing or unreadable file says nothing.
    pub(super) fn check_disk(&mut self) -> bool {
        let Some(loaded) = self.open.disk else { return false };
        let Some(now) = DiskStamp::of_source(&self.open.source.provenance) else { return false };
        if now == loaded {
            return false;
        }
        if self.mode != Mode::Browse || self.pending.is_some() {
            if self.open.disk_noticed == Some(now) {
                return false;
            }
            self.open.disk_noticed = Some(now);
            self.status = Some("File changed on disk: it reloads once you are done.".to_owned());
            return true;
        }
        if let Err(err) = self.reload() {
            self.open.disk_noticed = Some(now);
            self.status = Some(format!("File changed on disk; reload failed: {err:#}"));
            return true;
        }
        let reloaded = self.status.take().unwrap_or_default();
        self.status = Some(format!("File changed on disk \u{b7} {reloaded}"));
        true
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "tests assert by panicking")]

    use std::path::Path;
    use std::time::Duration;

    use crate::app::review_test_support::file_app;

    /// Rewrite `file_app`'s document with different content and a later mtime.
    fn edit_on_disk(root: &Path, app: &crate::app::App) {
        let path = root.join("docs/a.md");
        std::fs::write(&path, "# Plan\n\none\n\ntwo\n\nthree\n\nfour, from the agent\n").expect("rewrite");
        let later = app.open.disk.expect("stamp").modified + Duration::from_secs(5);
        std::fs::File::options().write(true).open(&path).expect("open").set_modified(later).expect("mtime");
    }

    #[test]
    fn an_unchanged_file_says_nothing() {
        let (root, mut app, _) = file_app("disk-unchanged");
        assert!(!app.check_disk());
        assert!(!app.poll_disk());
        assert_eq!(app.status, None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_change_on_disk_reloads_the_file_at_once() {
        let (root, mut app, _) = file_app("disk-changed");
        edit_on_disk(&root, &app);
        assert!(app.poll_disk());
        assert!(app.open.doc.source.contains("four, from the agent"), "reloaded without a key");
        let status = app.status.clone().expect("reload status");
        assert!(status.starts_with("File changed on disk \u{b7} reloaded"), "{status}");
        assert!(!app.check_disk(), "the new text is the loaded one now");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_change_waits_while_the_person_types_then_reloads() {
        let (root, mut app, _) = file_app("disk-typing");
        app.mode = super::super::Mode::Compose;
        edit_on_disk(&root, &app);
        assert!(app.check_disk());
        assert_eq!(app.status.as_deref(), Some("File changed on disk: it reloads once you are done."));
        assert!(!app.open.doc.source.contains("four, from the agent"), "not under the person's typing");
        assert!(!app.check_disk(), "said once for this change");
        app.mode = super::super::Mode::Browse;
        assert!(app.check_disk());
        assert!(app.open.doc.source.contains("four, from the agent"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_removed_file_says_nothing() {
        let (root, mut app, _) = file_app("disk-removed");
        std::fs::remove_file(root.join("docs/a.md")).expect("remove");
        assert!(!app.check_disk());
        assert_eq!(app.status, None);
        let _ = std::fs::remove_dir_all(root);
    }
}
