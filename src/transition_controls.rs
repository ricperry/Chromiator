//! GTK projection of document-owned transition settings; no independent recipe state.
use super::*;
use chromiator::document::TransitionProfile;

pub(super) struct TransitionControls {
    width: gtk::SpinButton,
    space: gtk::DropDown,
}

impl TransitionControls {
    pub(super) fn new(group: &gtk::Box) -> Self {
        let width = gtk::SpinButton::with_range(0.0, 100.0, 1.0);
        width.set_digits(1);
        width.set_numeric(true);
        width.set_width_chars(5);
        width.update_property(&[
            gtk::accessible::Property::Label("Transition width percent"),
            gtk::accessible::Property::Description("Color-space transition width from 0 to 100 percent; zero keeps hard boundaries between sites. This is not an image blur."),
        ]);
        width.connect_input(|spin| {
            chromiator::picker::preserve_displayed_precision(
                spin.text().as_str(), spin.value(), spin.digits(),
            ).map(Ok)
        });
        let space = gtk::DropDown::from_strings(&["Oklab", "Linear RGB"]);
        space.update_property(&[gtk::accessible::Property::Label("Transition blend space")]);
        group.append(&inspector_control_row("_Transition width", "Percent; 0 keeps hard boundaries", &width));
        group.append(&inspector_control_row("_Blend space", "Mix Target colors independently of Source matching; available when Transition width is above zero", &space));
        let controls = Self { width, space };
        controls.sync(TransitionProfile::HARD);
        controls
    }

    pub(super) fn sync(&self, profile: TransitionProfile) {
        self.width.set_value(f64::from(profile.width()) * 100.0);
        self.space.set_selected(match profile.blend_space { BlendSpace::Oklab => 0, BlendSpace::LinearRgb => 1 });
        self.update_sensitivity();
    }

    fn update_sensitivity(&self) {
        let enabled = self.width.value() > 0.0;
        self.space.set_sensitive(enabled);
    }
}

pub(super) fn connect(ui: &Rc<Ui>, state: &Rc<RefCell<State>>) {
    let view = ui.clone();
    let session = state.clone();
    ui.transitions.width.connect_value_changed(move |control| {
        if view.syncing.get() { return; }
        submit_session_edit(&view, &session,
            EditCommand::SetTransitionWidth((control.value() / 100.0) as f32),
            Some(EditGesture::TransitionWidth), "Transition width changed; updating preview...");
        view.transitions.update_sensitivity();
    });
    install_document_spin_boundaries(&ui.transitions.width, state, EditGesture::TransitionWidth);
    let view = ui.clone();
    let session = state.clone();
    ui.transitions.space.connect_selected_notify(move |control| {
        if view.syncing.get() || control.selected() == gtk::INVALID_LIST_POSITION { return; }
        let space = if control.selected() == 1 { BlendSpace::LinearRgb } else { BlendSpace::Oklab };
        submit_session_edit(&view, &session, EditCommand::SetBlendSpace(space), None,
            "Blend space changed; updating preview...");
    });
}
