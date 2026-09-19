//! Authoritative document editing, selection, savepoint, and history state.
//!
//! GTK code observes this session and submits typed commands. It never owns a second recipe or
//! history stack, which keeps dialog cancellation, no-op edits, preview scheduling, and undo/redo
//! independent from widget lifetime.

use thiserror::Error;

use crate::document::{BlendSpace, Document, Recipe, SampleSize, VoronoiMatching};
use crate::voronoi;

/// Stable site identifiers selected by the list, canvas, and detail disclosure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SiteSelection {
    /// Site used by canvas, list, color editing, and CLI aliases.
    pub selected: Option<u64>,
    /// Site whose inspector details are disclosed, when any.
    pub expanded: Option<u64>,
}

/// A gesture identity used to coalesce repeated widget changes into one undo step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditGesture {
    Smoothing,
    TransitionWidth,
    Influence(u64),
    /// A pointer drag that repeatedly resamples one source marker.
    Position(u64),
}

/// Complete creative mutations admitted by [`DocumentSession`].
#[derive(Clone, Debug, PartialEq)]
pub enum EditCommand {
    SetSmoothing(f32),
    SetHue(f32),
    SetMatching(VoronoiMatching),
    SetTransitionWidth(f32),
    SetBlendSpace(BlendSpace),
    SetSource {
        site_id: u64,
        color: [f32; 4],
        position: Option<[f64; 2]>,
    },
    SetTarget {
        site_id: u64,
        color: [f32; 3],
    },
    SetInfluence {
        site_id: u64,
        influence: f64,
    },
    SetSampleSize {
        site_id: u64,
        size: SampleSize,
    },
    Reattach {
        site_id: u64,
        position: [f64; 2],
    },
    SetLocked {
        site_id: u64,
        locked: bool,
    },
    AddSiteAt([f64; 2]),
    /// A picked color; Result-preview and picker colors have no attachment.
    AddSiteColor {
        color: [f32; 4],
        position: Option<[f64; 2]>,
    },
    DeleteSite(u64),
    ReplaceRecipe(Recipe),
}

/// Result of an admitted edit or history transition.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionChange {
    /// Whether the admitted transaction changed persisted creative state.
    pub changed: bool,
    /// Whether pixels can differ and a preview job must be scheduled.
    pub preview_required: bool,
    /// Whether the current recipe differs from the saved recipe.
    pub dirty: bool,
    /// Immutable recipe snapshot suitable for a worker job.
    pub recipe: Recipe,
    /// Stable selection after the transaction.
    pub selection: SiteSelection,
}

/// Admission failures that leave the session unchanged.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionError {
    #[error("no document is open")]
    NoDocument,
    #[error("invalid processing recipe: {0}")]
    InvalidRecipe(String),
}

#[derive(Clone, Debug, PartialEq)]
struct SessionSnapshot {
    recipe: Recipe,
    selection: SiteSelection,
}

/// Single authority for one open document and its creative transaction history.
#[derive(Default)]
pub struct DocumentSession {
    document: Option<Document>,
    selection: SiteSelection,
    undo: Vec<SessionSnapshot>,
    redo: Vec<SessionSnapshot>,
    saved_recipe: Option<Recipe>,
    active_gesture: Option<EditGesture>,
}

impl DocumentSession {
    /// Replace the open document and establish its current recipe as the saved baseline.
    pub fn load(&mut self, mut document: Document) -> Result<(), SessionError> {
        document
            .recipe
            .validate()
            .map_err(SessionError::InvalidRecipe)?;
        document.dirty = false;
        self.selection = SiteSelection {
            selected: document.recipe.voronoi.sites.first().map(|site| site.id),
            expanded: None,
        };
        self.saved_recipe = Some(document.recipe.clone());
        self.document = Some(document);
        self.undo.clear();
        self.redo.clear();
        self.active_gesture = None;
        Ok(())
    }

    /// Clears the active document and all associated session state.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Borrows the active document without cloning imported source bytes.
    pub fn document(&self) -> Option<&Document> {
        self.document.as_ref()
    }

    /// Clones a complete document only for a background job snapshot.
    pub fn document_cloned(&self) -> Option<Document> {
        self.document.clone()
    }

    /// Returns the stable selected and expanded site identifiers.
    pub fn selection(&self) -> SiteSelection {
        self.selection
    }

    /// Selects an existing site, optionally retargeting already-open details.
    pub fn select(&mut self, site_id: Option<u64>, preserve_expanded: bool) {
        let site_id = site_id.filter(|id| {
            self.document
                .as_ref()
                .is_some_and(|document| document.recipe.voronoi.site(*id).is_some())
        });
        self.selection.selected = site_id;
        if preserve_expanded && self.selection.expanded.is_some() {
            self.selection.expanded = site_id;
        }
    }

    /// Expands an existing site, or closes the detail disclosure with `None`.
    pub fn set_expanded(&mut self, site_id: Option<u64>) {
        self.selection.expanded = site_id.filter(|id| {
            self.document
                .as_ref()
                .is_some_and(|document| document.recipe.voronoi.site(*id).is_some())
        });
    }

    /// Returns whether a document undo step is available.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Returns whether a document redo step is available.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Returns the undo depth for diagnostics and deterministic audits.
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// Returns the redo depth for diagnostics and deterministic audits.
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// Returns whether the active recipe differs from its savepoint.
    pub fn is_dirty(&self) -> bool {
        self.document
            .as_ref()
            .is_some_and(|document| document.dirty)
    }

    /// Mark the current recipe saved without clearing creative undo/redo history.
    pub fn mark_saved(&mut self) {
        let Some(document) = self.document.as_mut() else {
            return;
        };
        document.dirty = false;
        self.saved_recipe = Some(document.recipe.clone());
        self.active_gesture = None;
    }

    /// Ends matching coalescing without modifying recipe or history.
    pub fn finish_gesture(&mut self, gesture: EditGesture) {
        if self.active_gesture == Some(gesture) {
            self.active_gesture = None;
        }
    }

    /// Validate and atomically commit one creative edit.
    ///
    /// Locked or value-identical edits return `changed = false`, create no history, and therefore
    /// must not schedule a preview. Invalid candidates leave the document untouched.
    pub fn edit(
        &mut self,
        command: EditCommand,
        gesture: Option<EditGesture>,
    ) -> Result<SessionChange, SessionError> {
        let before = self.snapshot().ok_or(SessionError::NoDocument)?;
        let document = self.document.as_ref().ok_or(SessionError::NoDocument)?;
        let mut recipe = document.recipe.clone();
        let source = document.source.clone();
        let details_were_open = self.selection.expanded.is_some();
        let mut replacement_selection: Option<Option<u64>> = None;
        let preview_required = !matches!(command, EditCommand::SetLocked { .. });
        let admitted = match command {
            EditCommand::SetSmoothing(value) => {
                if recipe.preprocessing.input_smoothing == value {
                    false
                } else {
                    recipe.preprocessing.input_smoothing = value;
                    true
                }
            }
            EditCommand::SetHue(value) => {
                if recipe.hue_degrees() == value {
                    false
                } else {
                    recipe.set_hue_degrees(value);
                    true
                }
            }
            EditCommand::SetMatching(value) => {
                if recipe.voronoi.matching == value {
                    false
                } else {
                    recipe.voronoi.matching = value;
                    true
                }
            }
            EditCommand::SetTransitionWidth(width) => {
                if !width.is_finite() || !(0.0..=1.0).contains(&width) {
                    return Err(SessionError::InvalidRecipe(
                        "Transition width must be finite and between 0 and 1".into(),
                    ));
                }
                recipe.voronoi.transition = recipe.voronoi.transition.with_width(width);
                true
            }
            EditCommand::SetBlendSpace(space) => {
                recipe.voronoi.transition.blend_space = space;
                true
            }
            EditCommand::SetSource {
                site_id,
                color,
                position,
            } => recipe.voronoi.set_source(site_id, color, position),
            EditCommand::SetTarget { site_id, color } => recipe.voronoi.set_target(site_id, color),
            EditCommand::SetInfluence { site_id, influence } => {
                recipe.voronoi.set_influence(site_id, influence)
            }
            EditCommand::SetSampleSize { site_id, size } => {
                let sample = recipe.voronoi.site(site_id).and_then(|site| {
                    (!site.locked)
                        .then_some(site.position)
                        .flatten()
                        .and_then(|position| voronoi::sample_color(&source, position, size))
                });
                sample.is_some_and(|color| recipe.voronoi.set_size(site_id, size, color))
            }
            EditCommand::Reattach { site_id, position } => {
                voronoi::reattach_site(&mut recipe.voronoi, &source, site_id, position)
            }
            EditCommand::SetLocked { site_id, locked } => {
                recipe.voronoi.set_locked(site_id, locked)
            }
            EditCommand::AddSiteAt(position) => {
                let selected = voronoi::add_site_at(&mut recipe.voronoi, &source, position);
                replacement_selection = selected.map(Some);
                selected.is_some()
            }
            EditCommand::AddSiteColor { color, position } => {
                let id = voronoi::add_site_color(&mut recipe.voronoi, color, position);
                replacement_selection = Some(Some(id));
                true
            }
            EditCommand::DeleteSite(site_id) => {
                let deleted = recipe.voronoi.delete_site(site_id);
                if deleted {
                    replacement_selection = Some(recipe.voronoi.sites.first().map(|site| site.id));
                }
                deleted
            }
            EditCommand::ReplaceRecipe(replacement) => {
                if recipe == replacement {
                    false
                } else {
                    recipe = replacement;
                    replacement_selection = Some(recipe.voronoi.sites.first().map(|site| site.id));
                    true
                }
            }
        };
        let changed = admitted && recipe != before.recipe;
        if !changed {
            return Ok(self.outcome(false, false));
        }
        recipe.validate().map_err(SessionError::InvalidRecipe)?;
        if let Some(selected) = replacement_selection {
            self.selection.selected = selected;
            if details_were_open {
                self.selection.expanded = selected;
            }
        }
        self.record(before, &recipe, gesture);
        let dirty = self.saved_recipe.as_ref() != Some(&recipe);
        let document = self.document.as_mut().ok_or(SessionError::NoDocument)?;
        document.recipe = recipe;
        document.dirty = dirty;
        Ok(self.outcome(true, preview_required))
    }

    /// Applies one undo transition, returning `changed = false` at the boundary.
    pub fn undo(&mut self) -> Result<SessionChange, SessionError> {
        self.history_step(false)
    }

    /// Applies one redo transition, returning `changed = false` at the boundary.
    pub fn redo(&mut self) -> Result<SessionChange, SessionError> {
        self.history_step(true)
    }

    fn history_step(&mut self, redo: bool) -> Result<SessionChange, SessionError> {
        let now = self.snapshot().ok_or(SessionError::NoDocument)?;
        self.active_gesture = None;
        let target = if redo {
            let Some(target) = self.redo.pop() else {
                return Ok(self.outcome(false, false));
            };
            self.undo.push(now);
            target
        } else {
            let Some(target) = self.undo.pop() else {
                return Ok(self.outcome(false, false));
            };
            self.redo.push(now);
            target
        };
        let dirty = self.saved_recipe.as_ref() != Some(&target.recipe);
        let document = self.document.as_mut().ok_or(SessionError::NoDocument)?;
        document.recipe = target.recipe;
        document.dirty = dirty;
        self.selection = target.selection;
        Ok(self.outcome(true, true))
    }

    fn snapshot(&self) -> Option<SessionSnapshot> {
        Some(SessionSnapshot {
            recipe: self.document.as_ref()?.recipe.clone(),
            selection: self.selection,
        })
    }

    fn record(&mut self, before: SessionSnapshot, recipe: &Recipe, gesture: Option<EditGesture>) {
        if self.active_gesture == gesture
            && gesture.is_some()
            && self
                .undo
                .last()
                .is_some_and(|saved| saved.recipe == *recipe)
        {
            self.undo.pop();
            self.active_gesture = None;
        } else if self.active_gesture != gesture || gesture.is_none() {
            self.undo.push(before);
            self.active_gesture = gesture;
        }
        self.redo.clear();
    }

    fn outcome(&self, changed: bool, preview_required: bool) -> SessionChange {
        SessionChange {
            changed,
            preview_required,
            dirty: self.is_dirty(),
            recipe: self
                .document
                .as_ref()
                .map_or_else(Recipe::default, |document| document.recipe.clone()),
            selection: self.selection,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::starter_looks::{STARTER_LOOKS, recipe_for_starter_look};

    fn session() -> DocumentSession {
        let mut document = crate::example::spectrum_document().unwrap();
        document.recipe = recipe_for_starter_look(STARTER_LOOKS[0]);
        let mut session = DocumentSession::default();
        session.load(document).unwrap();
        session
    }

    #[test]
    fn transactions_coalesce_restore_savepoints_and_reject_invalid_candidates() {
        let mut session = session();
        assert!(!session.is_dirty());
        assert!(
            session
                .edit(EditCommand::SetSmoothing(1.0), Some(EditGesture::Smoothing))
                .unwrap()
                .changed
        );
        session
            .edit(EditCommand::SetSmoothing(2.0), Some(EditGesture::Smoothing))
            .unwrap();
        assert_eq!(session.undo_len(), 1);
        session.finish_gesture(EditGesture::Smoothing);
        assert_eq!(
            session.undo().unwrap().recipe.preprocessing.input_smoothing,
            0.0
        );
        assert_eq!(
            session.redo().unwrap().recipe.preprocessing.input_smoothing,
            2.0
        );
        session.mark_saved();
        assert!(!session.is_dirty());
        assert_eq!(session.undo_len(), 1, "saving must retain creative history");
        let before = session.document().unwrap().recipe.clone();
        assert!(matches!(
            session.edit(EditCommand::SetSmoothing(f32::NAN), None),
            Err(SessionError::InvalidRecipe(_))
        ));
        assert_eq!(session.document().unwrap().recipe, before);
        assert!(!session.is_dirty());
    }

    #[test]
    fn noops_locks_and_picker_style_commit_create_only_real_global_history() {
        let mut session = session();
        let site_id = session.selection().selected.unwrap();
        let target = session
            .document()
            .unwrap()
            .recipe
            .voronoi
            .site(site_id)
            .unwrap()
            .target_color;
        assert!(
            !session
                .edit(
                    EditCommand::SetTarget {
                        site_id,
                        color: target
                    },
                    None
                )
                .unwrap()
                .changed
        );
        assert_eq!(session.undo_len(), 0);
        let lock = session
            .edit(
                EditCommand::SetLocked {
                    site_id,
                    locked: true,
                },
                None,
            )
            .unwrap();
        assert!(lock.changed);
        assert!(!lock.preview_required);
        let after_lock = session.undo_len();
        assert!(
            !session
                .edit(
                    EditCommand::SetTarget {
                        site_id,
                        color: [0.1, 0.2, 0.3]
                    },
                    None
                )
                .unwrap()
                .changed
        );
        assert_eq!(session.undo_len(), after_lock);
        session
            .edit(
                EditCommand::SetLocked {
                    site_id,
                    locked: false,
                },
                None,
            )
            .unwrap();
        let before_commit = session.undo_len();
        let selected = [0.1, 0.2, 0.3];
        assert!(
            session
                .edit(
                    EditCommand::SetTarget {
                        site_id,
                        color: selected
                    },
                    None
                )
                .unwrap()
                .changed
        );
        assert_eq!(session.undo_len(), before_commit + 1);
        assert!(
            !session
                .edit(
                    EditCommand::SetTarget {
                        site_id,
                        color: selected
                    },
                    None
                )
                .unwrap()
                .changed
        );
        assert_eq!(session.undo_len(), before_commit + 1);
    }

    #[test]
    fn abandoned_picker_draft_never_enters_document_history() {
        use crate::color::DraftColor;
        use crate::picker::{PickerGesture, PickerLocalHistory};

        let session = session();
        let recipe = session.document().unwrap().recipe.clone();
        let mut local = PickerLocalHistory::default();
        let before = DraftColor::new([0.1, 0.2, 0.3]);
        let after = DraftColor::new([0.4, 0.5, 0.6]);
        assert!(local.record(before, after, Some(PickerGesture::Wheel)));
        drop(local);
        assert_eq!(session.document().unwrap().recipe, recipe);
        assert_eq!(session.undo_len(), 0);
    }

    #[test]
    fn add_delete_and_recipe_replacement_keep_selection_on_stable_site_ids() {
        let mut session = session();
        session.set_expanded(session.selection().selected);
        let added = session
            .edit(EditCommand::AddSiteAt([0.5, 0.5]), None)
            .unwrap();
        assert!(added.changed);
        let added_id = added.selection.selected.unwrap();
        assert_eq!(added.selection.expanded, Some(added_id));
        let deleted = session
            .edit(EditCommand::DeleteSite(added_id), None)
            .unwrap();
        assert!(deleted.changed);
        assert_ne!(deleted.selection.selected, Some(added_id));

        let replacement = recipe_for_starter_look(STARTER_LOOKS[1]);
        let replaced = session
            .edit(EditCommand::ReplaceRecipe(replacement.clone()), None)
            .unwrap();
        assert!(replaced.changed);
        assert_eq!(replaced.recipe, replacement);
        assert_eq!(
            replaced.selection.selected,
            replacement.voronoi.sites.first().map(|site| site.id)
        );
    }
}
