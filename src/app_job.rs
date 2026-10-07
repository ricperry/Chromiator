//! Widget-independent messages exchanged with file and processing workers.

use std::path::PathBuf;

use crate::document::{Document, PixelImage};
use crate::preset::Preset;
use crate::processing::{Coverage, DisplayBuffer};

/// Fully decoded document and its presentation buffers admitted by an open job.
pub type PreparedDocument = (
    Document,
    Option<PathBuf>,
    DocumentKind,
    PixelImage,
    DisplayBuffer,
    DisplayBuffer,
    Coverage,
);

/// Result payload delivered by an open worker.
pub type OpenResult = Result<PreparedDocument, String>;

/// A source-free processing recipe extracted from a project or preset file.
pub type PresetLoadResult = Result<Preset, String>;

/// Typed events sent from worker threads to the GTK main loop.
pub enum Work {
    Progress(u64, f64, &'static str),
    Open(u64, DocumentKind, Box<OpenResult>),
    PresetLoaded(u64, Box<PresetLoadResult>),
    Save(u64, Result<PathBuf, String>),
    Export(u64, Result<PathBuf, String>),
}

/// Source of the active document, used for status and error presentation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DocumentKind {
    #[default]
    Welcome,
    Image,
    Example,
    Project,
}
