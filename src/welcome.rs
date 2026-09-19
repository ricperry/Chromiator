//! Startup recent-project metadata and projection. Project contents stay with file jobs.
use super::*;
use std::io::{Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const RECENT_LIMIT: usize = 12;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct RecentProject {
    path: PathBuf,
    used_at: u64,
}

pub(super) struct RecentProjects {
    path: PathBuf,
    entries: Vec<RecentProject>,
}

impl RecentProjects {
    pub(super) fn load() -> Self {
        let path = glib::user_data_dir().join("chromiator").join("recent-projects.json");
        let mut store = Self { path, entries: Vec::new() };
        match store.read() {
            Ok(entries) => store.entries = entries,
            Err(error) => eprintln!("Could not read recent-project history {}: {error:#}", store.path.display()),
        }
        store
    }

    fn read(&self) -> anyhow::Result<Vec<RecentProject>> {
        let file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        file.take(1_048_577).read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() <= 1_048_576, "recent-project history exceeds 1 MiB");
        let mut entries: Vec<RecentProject> = serde_json::from_slice(&bytes)?;
        entries.retain(|entry| entry.path.is_absolute());
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.used_at));
        let mut seen = std::collections::HashSet::new();
        entries.retain(|entry| seen.insert(entry.path.clone()));
        entries.truncate(RECENT_LIMIT);
        Ok(entries)
    }

    fn persist(&self) -> anyhow::Result<()> {
        let parent = self.path.parent().expect("history path has a parent");
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut file, &self.entries)?;
        file.flush()?;
        file.as_file().sync_all()?;
        file.persist(&self.path)?;
        Ok(())
    }
}

pub(super) fn connect(ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
    populate(ui, state);
    let weak_ui = Rc::downgrade(ui);
    let state = state.clone();
    ui.recent_clear.connect_clicked(move |_| {
        let Some(ui) = weak_ui.upgrade() else { return; };
        let result = {
            let mut store = ui.recent_store.borrow_mut();
            let previous = std::mem::take(&mut store.entries);
            let result = store.persist();
            if result.is_err() { store.entries = previous; }
            result
        };
        if let Err(error) = result {
            self::error(&ui, "Could not clear recent projects", &format!("{error:#}"));
        }
        populate(&ui, &state);
    });
}

/// Called only after a current open/save result has been admitted successfully.
pub(super) fn remember(ui: &Rc<Ui>, state: &Rc<RefCell<State>>, path: &Path) {
    let result = (|| -> anyhow::Result<()> {
        let path = std::fs::canonicalize(path)?;
        let mut store = ui.recent_store.borrow_mut();
        store.entries.retain(|entry| entry.path != path);
        store.entries.insert(0, RecentProject {
            path,
            used_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        });
        store.entries.truncate(RECENT_LIMIT);
        store.persist()
    })();
    if let Err(error) = result {
        eprintln!("Could not update recent-project history for {}: {error:#}", path.display());
    }
    populate(ui, state);
}

fn populate(ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
    while let Some(child) = ui.recent_projects.first_child() {
        ui.recent_projects.remove(&child);
    }
    let entries = ui.recent_store.borrow().entries.clone();
    ui.recent_clear.set_sensitive(!entries.is_empty());
    ui.recent_clear.set_visible(!entries.is_empty());
    if entries.is_empty() {
        let label = gtk::Label::new(Some("No recent projects yet. Open or save a project to find it here next time."));
        label.set_wrap(true);
        label.set_max_width_chars(42);
        label.set_xalign(0.0);
        label.add_css_class("dim-label");
        ui.recent_projects.append(&label);
    }
    for entry in entries {
        let name = entry.path.file_name().unwrap_or_default().to_string_lossy();
        let button = gtk::Button::new();
        button.add_css_class("flat");
        button.add_css_class("chromiator-recent-project");
        let labels = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let title = gtk::Label::new(Some(&name));
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        title.set_max_width_chars(38);
        labels.append(&title);
        let folder = gtk::Label::new(Some(&entry.path.parent().unwrap_or_else(|| Path::new("")).to_string_lossy()));
        folder.set_xalign(0.0);
        folder.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        folder.set_max_width_chars(38);
        folder.add_css_class("dim-label");
        folder.add_css_class("caption");
        labels.append(&folder);
        if let Some(date) = i64::try_from(entry.used_at).ok()
            .and_then(|seconds| glib::DateTime::from_unix_local(seconds).ok())
            .and_then(|date| date.format("%b %e, %Y %H:%M").ok()) {
            let date = gtk::Label::new(Some(date.as_str()));
            date.set_xalign(0.0);
            date.add_css_class("dim-label");
            date.add_css_class("caption");
            labels.append(&date);
        }
        button.set_child(Some(&labels));
        button.set_tooltip_text(Some(&entry.path.to_string_lossy()));
        button.update_property(&[gtk::accessible::Property::Label(&format!("Open {name}"))]);
        let weak_ui = Rc::downgrade(ui);
        let state = state.clone();
        button.connect_clicked(move |_| {
            if let Some(ui) = weak_ui.upgrade() {
                shell_actions::open_recent_project(&ui, &state, entry.path.clone());
            }
        });
        ui.recent_projects.append(&button);
    }
}
