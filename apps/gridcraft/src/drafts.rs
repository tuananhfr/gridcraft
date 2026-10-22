use gridcraft_ui_egui::now_ms;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::drafts::{self, Planner};

const MAX_COUNT: usize = 10;
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_NAME: usize = 4096;

pub struct Drafts {
    dir: PathBuf,
    planner: Planner,
    root: PathBuf,
    _lock: std::fs::File,
}

impl Drafts {
    pub fn new(root: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&root)?;
        let prefix = format!("{}-{}-", std::process::id(), now_ms() as u64);
        for _ in 0..5 {
            let pending = tempfile::Builder::new().prefix(&prefix).suffix(".lock").tempfile_in(&root)?;
            if pending.as_file().try_lock().is_err() {
                continue;
            }
            let dir = pending.path().with_extension("");
            let (lock, lock_path) = pending.keep().map_err(|e| e.error)?;
            if let Err(e) = std::fs::create_dir(&dir) {
                drop(lock);
                let _ = std::fs::remove_file(lock_path);
                return Err(e);
            }
            return Ok(Self { dir, planner: Planner::default(), root, _lock: lock });
        }
        Err(std::io::Error::other("could not lock recovery draft store"))
    }

    fn recover_dead(&self, app: &mut SheetApp) -> std::io::Result<()> {
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            if path.extension().is_none_or(|e| e != "lock") || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                continue;
            }
            let dir = self.root.join(id);
            if dir == self.dir {
                continue;
            }
            let Ok(lock) = std::fs::OpenOptions::new().read(true).write(true).open(&path) else { continue };
            if lock.try_lock().is_err() {
                continue;
            }
            if !dir.exists() {
                let old = lock.metadata()?.modified()?.elapsed().is_ok_and(|age| age > std::time::Duration::from_secs(60));
                drop(lock);
                if old {
                    let _ = std::fs::remove_file(path);
                }
                continue;
            }
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                if entry.file_type()?.is_file() && entry.file_name().to_string_lossy().starts_with(".draft-write-") {
                    std::fs::remove_file(entry.path())?;
                }
            }
            for source in Self::entries_in(&dir)? {
                let result = (|| {
                    let (name, bytes) = Self::read(&source)?;
                    gridcraft_engine::io::open_bytes(&name, &bytes).map_err(std::io::Error::other)?;
                    let key =
                        format!("recovered-{}", tempfile::NamedTempFile::new_in(&self.dir)?.path().file_name().unwrap_or_default().to_string_lossy());
                    self.put(&key, &name, &bytes)?;
                    std::fs::remove_file(&source)
                })();
                if let Err(e) = result {
                    report(app, format!("{}: {}: {e}", app.ui.language.tr("Could not restore draft"), source.display()));
                }
            }
            let empty = std::fs::read_dir(&dir)?.next().is_none();
            if empty {
                std::fs::remove_dir(&dir)?;
            }
            // The lock lives outside the directory so Windows can remove the empty directory while it is held.
            drop(lock);
            if empty {
                let _ = std::fs::remove_file(path);
            }
        }
        Ok(())
    }

    fn entries(&self) -> std::io::Result<Vec<PathBuf>> {
        Self::entries_in(&self.dir)
    }

    fn entries_in(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() && entry.path().extension().is_some_and(|e| e == "draft") {
                paths.push(entry.path());
            }
        }
        paths.sort();
        Ok(paths)
    }

    fn put(&self, key: &str, name: &str, bytes: &[u8]) -> std::io::Result<()> {
        if name.len() > MAX_NAME || name.contains('\n') || bytes.len() as u64 > MAX_BYTES.saturating_sub(name.len() as u64 + 1) {
            return Err(std::io::Error::other("recovery draft exceeds size limit"));
        }
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(format!("{key}.draft"));
        let others: Vec<_> = self.entries()?.into_iter().filter(|p| *p != path).collect();
        let mut total = bytes.len() as u64 + name.len() as u64 + 1;
        for p in &others {
            total = total.saturating_add(std::fs::metadata(p)?.len());
        }
        // Keep existing recovery work when the store is full.
        if others.len() >= MAX_COUNT || total > MAX_BYTES {
            return Err(std::io::Error::other("recovery draft store is full"));
        }
        let mut file = tempfile::Builder::new().prefix(".draft-write-").tempfile_in(&self.dir)?;
        writeln!(file, "{name}")?;
        file.write_all(bytes)?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|e| e.error)?;
        Ok(())
    }

    fn read(path: &Path) -> std::io::Result<(String, Vec<u8>)> {
        let file = std::fs::File::open(path)?;
        if file.metadata()?.len() > MAX_BYTES {
            return Err(std::io::Error::other("recovery draft exceeds size limit"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(std::io::Error::other("recovery draft exceeds size limit"));
        }
        let split =
            bytes.iter().position(|b| *b == b'\n').filter(|n| *n <= MAX_NAME).ok_or_else(|| std::io::Error::other("invalid recovery draft"))?;
        let name = std::str::from_utf8(bytes.get(..split).unwrap_or_default()).map_err(std::io::Error::other)?.to_string();
        let payload = bytes.get(split.saturating_add(1)..).unwrap_or_default().to_vec();
        Ok((name, payload))
    }

    pub fn restore(&mut self, app: &mut SheetApp) {
        if let Err(e) = self.recover_dead(app) {
            report(app, format!("{}: {e}", app.ui.language.tr("Could not restore draft")));
        }
        let paths = match self.entries() {
            Ok(paths) => paths,
            Err(e) => {
                report(app, format!("{}: {e}", app.ui.language.tr("Could not restore draft")));
                return;
            }
        };
        let mut total = 0u64;
        for path in paths.into_iter().take(MAX_COUNT) {
            let result = Self::read(&path).and_then(|(name, bytes)| {
                total = total.saturating_add(bytes.len() as u64);
                if total > MAX_BYTES {
                    return Err(std::io::Error::other("recovery draft store exceeds size limit"));
                }
                let uid = app.open_draft(&name, &bytes).ok_or_else(|| std::io::Error::other("could not open workbook"))?;
                if let Some(d) = app.session.documents().iter().find(|d| d.uid == uid) {
                    let key = path.file_stem().and_then(|s| s.to_str()).ok_or_else(|| std::io::Error::other("invalid draft key"))?;
                    self.planner.adopt(uid, key, &d.wb, 0.0);
                }
                if let Some(d) = app.session.active_mut() {
                    let path = Path::new(&name);
                    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Book");
                    d.title = format!("{stem} ({}).{}", app.ui.language.tr("Recovered"), drafts::draft_format(&name));
                }
                app.toast = Some((app.ui.language.tr("Recovered").to_string(), gridcraft_ui_egui::now_ms()));
                Ok(())
            });
            if let Err(e) = result {
                report(app, format!("{}: {}: {e}", app.ui.language.tr("Could not restore draft"), path.display()));
            }
        }
    }

    pub fn discard_on_exit(&mut self) {
        for key in self.planner.plan(&[], now_ms(), false).remove {
            if let Err(e) = std::fs::remove_file(self.dir.join(format!("{key}.draft")))
                && e.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!("could not remove recovery draft: {e}");
            }
        }
    }

    pub fn after_frame(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        let now = now_ms();
        let docs = app.draft_docs();
        let plan = self.planner.plan(&docs, now, app.editing_in_place());
        for key in plan.remove {
            if let Err(e) = std::fs::remove_file(self.dir.join(format!("{key}.draft")))
                && e.kind() != std::io::ErrorKind::NotFound
            {
                report(app, e.to_string());
            }
        }
        for d in docs.iter().filter(|d| plan.put.contains(&d.uid)) {
            let start = now_ms();
            let key = self.planner.key(d.uid);
            let key = if key.starts_with("doc-") { format!("draft-{}-{}-{}", std::process::id(), now as u64, d.uid) } else { key };
            let result = drafts::draft_bytes(&d.wb, &d.name).and_then(|bytes| self.put(&key, &d.name, &bytes).map_err(|e| e.to_string()));
            if let Err(e) = result {
                report(app, format!("{}: {e}", app.ui.language.tr("Could not save recovery draft")));
            }
            self.planner.adopt(d.uid, &key, &d.wb, now);
            self.planner.record_put(d.uid, &d.wb, now, now_ms() - start);
            app.show_draft_warnings(d.uid, &gridcraft_engine::io::save_warnings(&d.wb, &d.name));
        }
        if plan.waiting {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
    }
}

fn report(app: &mut SheetApp, message: String) {
    app.message = Some((app.ui.language.tr("Recovery").to_string(), message));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_store_roundtrip_caps_restore_and_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let store = Drafts::new(dir.path().join("drafts")).unwrap();
        let wb = gridcraft_engine::model::Workbook::new();
        let bytes = drafts::draft_bytes(&wb, "Book.xlsx").unwrap();
        store.put("doc-1", "Book.xlsx", &bytes).unwrap();
        assert_eq!(Drafts::read(&store.entries().unwrap()[0]).unwrap(), ("Book.xlsx".into(), bytes.clone()));
        assert!(store.put("large", "x", &vec![0; MAX_BYTES as usize]).is_err());
        for n in 2..=MAX_COUNT {
            store.put(&format!("doc-{n}"), "Book.xlsx", &bytes).unwrap();
        }
        assert!(store.put("overflow", "Book.xlsx", &bytes).is_err());
        let root = store.root.clone();
        std::fs::write(store.dir.join(".draft-write-stale"), b"partial").unwrap();
        drop(store);
        let mut store = Drafts::new(root).unwrap();
        let mut app = SheetApp::new(gridcraft_engine::Session::new(), Default::default());
        store.restore(&mut app);
        assert_eq!(app.session.documents().len(), MAX_COUNT);
        assert!(app.session.documents().iter().all(|d| d.is_dirty() && d.path.is_none() && d.title.contains("Recovered")));
        if let Some(d) = app.session.active_mut() {
            d.saved = d.wb.clone();
        }
        store.after_frame(&mut app, &egui::Context::default());
        assert_eq!(store.entries().unwrap().len(), MAX_COUNT - 1);
        app.session = gridcraft_engine::Session::new();
        store.after_frame(&mut app, &egui::Context::default());
        assert!(store.entries().unwrap().is_empty());
    }

    #[test]
    fn live_instances_are_isolated_and_dead_drafts_are_adopted() {
        let root = tempfile::tempdir().unwrap();
        let first = Drafts::new(root.path().to_path_buf()).unwrap();
        let bytes = drafts::draft_bytes(&gridcraft_engine::model::Workbook::new(), "Book.xlsx").unwrap();
        first.put("live", "Book.xlsx", &bytes).unwrap();
        let first_dir = first.dir.clone();
        let first_lock = first_dir.with_extension("lock");
        let mut second = Drafts::new(root.path().to_path_buf()).unwrap();
        let mut app = SheetApp::new(gridcraft_engine::Session::new(), Default::default());
        second.restore(&mut app);
        assert!(second.entries().unwrap().is_empty());
        assert!(app.session.documents().is_empty());
        second.planner.adopt(999, "live", &std::sync::Arc::new(gridcraft_engine::model::Workbook::new()), 0.0);
        second.after_frame(&mut app, &egui::Context::default());
        second.planner.adopt(999, "live", &std::sync::Arc::new(gridcraft_engine::model::Workbook::new()), 0.0);
        second.discard_on_exit();
        assert!(first_dir.join("live.draft").exists());
        second.put("second-live", "Book.xlsx", &bytes).unwrap();
        drop(first);
        let mut third = Drafts::new(root.path().to_path_buf()).unwrap();
        third.restore(&mut app);
        assert_eq!(app.session.documents().len(), 1);
        assert!(!first_dir.exists());
        assert!(!first_lock.exists());
        assert!(second.dir.join("second-live.draft").exists());
        third.after_frame(&mut app, &egui::Context::default());
        third.discard_on_exit();
        assert!(third.entries().unwrap().is_empty());
        assert_eq!(Drafts::read(&second.dir.join("second-live.draft")).unwrap().1, bytes);
    }

    #[test]
    fn recovery_keeps_drafts_that_exceed_our_store_limits() {
        let root = tempfile::tempdir().unwrap();
        let dead = Drafts::new(root.path().to_path_buf()).unwrap();
        let bytes = drafts::draft_bytes(&gridcraft_engine::model::Workbook::new(), "Book.xlsx").unwrap();
        dead.put("pending", "Book.xlsx", &bytes).unwrap();
        let dead_dir = dead.dir.clone();
        drop(dead);
        let store = Drafts::new(root.path().to_path_buf()).unwrap();
        for n in 0..MAX_COUNT {
            store.put(&format!("own-{n}"), "Book.xlsx", &bytes).unwrap();
        }
        let mut app = SheetApp::new(gridcraft_engine::Session::new(), Default::default());
        store.recover_dead(&mut app).unwrap();
        assert!(app.message.is_some());
        assert_eq!(store.entries().unwrap().len(), MAX_COUNT);
        assert!(dead_dir.join("pending.draft").exists());
        assert!(dead_dir.with_extension("lock").exists());
        for path in store.entries().unwrap() {
            std::fs::remove_file(path).unwrap();
        }
        let full = std::fs::File::create_new(store.dir.join("full.draft")).unwrap();
        full.set_len(MAX_BYTES).unwrap();
        drop(full);
        assert!(store.put("extra", "Book.xlsx", &bytes).is_err());
        store.recover_dead(&mut app).unwrap();
        assert!(dead_dir.join("pending.draft").exists());
    }

    #[test]
    fn orphan_locks_respect_startup_grace_period() {
        let root = tempfile::tempdir().unwrap();
        let fresh = root.path().join("fresh.lock");
        let old = root.path().join("old.lock");
        std::fs::File::create_new(&fresh).unwrap();
        let file = std::fs::File::create_new(&old).unwrap();
        file.set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1)).unwrap();
        drop(file);
        let ignored = root.path().join("unlocked");
        std::fs::create_dir(&ignored).unwrap();
        std::fs::write(ignored.join("untouched.draft"), b"invalid").unwrap();
        let mut store = Drafts::new(root.path().to_path_buf()).unwrap();
        let mut app = SheetApp::new(gridcraft_engine::Session::new(), Default::default());
        store.restore(&mut app);
        assert!(fresh.exists());
        assert!(!old.exists());
        assert!(ignored.join("untouched.draft").exists());
    }

    #[test]
    fn macro_draft_restore_keeps_vba() {
        let dir = tempfile::tempdir().unwrap();
        let store = Drafts::new(dir.path().to_path_buf()).unwrap();
        let mut wb = gridcraft_engine::model::Workbook::new();
        wb.vba_project = Some(b"synthetic VBA".to_vec());
        for ext in ["xlsm", "xltm"] {
            let name = format!("Macro.{ext}");
            let bytes = drafts::draft_bytes(&wb, &name).unwrap();
            store.put(ext, &name, &bytes).unwrap();
        }
        let root = store.root.clone();
        drop(store);
        let mut store = Drafts::new(root).unwrap();
        let mut app = SheetApp::new(gridcraft_engine::Session::new(), Default::default());
        store.restore(&mut app);
        assert_eq!(app.session.documents().len(), 2);
        for d in app.session.documents() {
            assert_eq!(d.wb.vba_project, wb.vba_project);
            assert!(!gridcraft_engine::io::drops_macros(&d.wb, &d.title));
        }
    }

    #[test]
    fn corrupt_restore_is_reported_and_retained() {
        let dir = tempfile::tempdir().unwrap();
        let store = Drafts::new(dir.path().to_path_buf()).unwrap();
        store.put("doc-1", "Book.xlsx", b"PK\x03\x04broken").unwrap();
        std::fs::write(store.dir.join("invalid.draft"), b"invalid header").unwrap();
        let dead_dir = store.dir.clone();
        let root = store.root.clone();
        drop(store);
        let mut store = Drafts::new(root).unwrap();
        let mut app = SheetApp::new(gridcraft_engine::Session::new(), Default::default());
        store.restore(&mut app);
        assert!(app.message.is_some());
        assert!(dead_dir.join("doc-1.draft").exists());
        assert!(dead_dir.join("invalid.draft").exists());
        assert!(dead_dir.with_extension("lock").exists());
        assert!(store.entries().unwrap().is_empty());
    }
}
