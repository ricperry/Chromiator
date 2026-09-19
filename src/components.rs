//! Resource-defined GTK application shell with no document or history authority.

use gtk::glib;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/chromiator/Chromiator/window.ui")]
    pub struct ChromiatorMainShell {
        #[template_child]
        pub header: gtk::TemplateChild<gtk::HeaderBar>,
        #[template_child]
        pub page_stack: gtk::TemplateChild<gtk::Stack>,
        #[template_child]
        pub workspace_split: gtk::TemplateChild<gtk::Paned>,
        #[template_child]
        pub inspector_scroll: gtk::TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub inspector_content: gtk::TemplateChild<gtk::Box>,
        #[template_child]
        pub welcome_media: gtk::TemplateChild<gtk::Box>,
        #[template_child]
        pub welcome_recent: gtk::TemplateChild<gtk::Box>,
        #[template_child]
        pub welcome_recent_clear: gtk::TemplateChild<gtk::Button>,
        #[template_child]
        pub canvas: gtk::TemplateChild<gtk::DrawingArea>,
        #[template_child]
        pub comparison_controls: gtk::TemplateChild<gtk::Box>,
        #[template_child]
        pub result_mode: gtk::TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub split_mode: gtk::TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub source_mode: gtk::TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub open_button: gtk::TemplateChild<gtk::Button>,
        #[template_child]
        pub save_button: gtk::TemplateChild<gtk::Button>,
        #[template_child]
        pub undo_button: gtk::TemplateChild<gtk::Button>,
        #[template_child]
        pub redo_button: gtk::TemplateChild<gtk::Button>,
        #[template_child]
        pub export_button: gtk::TemplateChild<gtk::Button>,
        #[template_child]
        pub document_menu: gtk::TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub sidebar_button: gtk::TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub file_spacer: gtk::TemplateChild<gtk::Separator>,
        #[template_child]
        pub history_spacer: gtk::TemplateChild<gtk::Separator>,
        #[template_child]
        pub welcome_open: gtk::TemplateChild<gtk::Button>,
        #[template_child]
        pub welcome_project: gtk::TemplateChild<gtk::Button>,
        #[template_child]
        pub welcome_example: gtk::TemplateChild<gtk::Button>,
        #[template_child]
        pub status_bar: gtk::TemplateChild<gtk::Box>,
        #[template_child]
        pub status: gtk::TemplateChild<gtk::Label>,
        #[template_child]
        pub pipeline_status: gtk::TemplateChild<gtk::Label>,
        #[template_child]
        pub progress: gtk::TemplateChild<gtk::ProgressBar>,
        #[template_child]
        pub cancel: gtk::TemplateChild<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ChromiatorMainShell {
        const NAME: &'static str = "ChromiatorMainShell";
        type Type = super::ChromiatorMainShell;
        type ParentType = gtk::Box;

        fn class_init(class: &mut Self::Class) {
            Self::bind_template(class);
        }
        fn instance_init(object: &glib::subclass::InitializingObject<Self>) {
            object.init_template();
        }
    }

    impl ObjectImpl for ChromiatorMainShell {}
    impl WidgetImpl for ChromiatorMainShell {}
    impl BoxImpl for ChromiatorMainShell {}
}

glib::wrapper! {
    /// Static main-window composition shared with Toniator's native GTK visual language.
    pub struct ChromiatorMainShell(ObjectSubclass<imp::ChromiatorMainShell>)
        @extends gtk::Widget, gtk::Box,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl ChromiatorMainShell {
    /// Instantiates the registered resource template.
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    /// Returns the custom header for installation in the window's titlebar slot.
    pub fn header(&self) -> gtk::HeaderBar {
        self.imp().header.get()
    }
    pub fn page_stack(&self) -> gtk::Stack {
        self.imp().page_stack.get()
    }
    pub fn split(&self) -> gtk::Paned {
        self.imp().workspace_split.get()
    }
    pub fn inspector_scroll(&self) -> gtk::ScrolledWindow {
        self.imp().inspector_scroll.get()
    }
    pub fn inspector_content(&self) -> gtk::Box {
        self.imp().inspector_content.get()
    }
    pub fn welcome_media(&self) -> gtk::Box {
        self.imp().welcome_media.get()
    }
    pub fn welcome_recent(&self) -> gtk::Box {
        self.imp().welcome_recent.get()
    }
    pub fn welcome_recent_clear(&self) -> gtk::Button {
        self.imp().welcome_recent_clear.get()
    }
    pub fn canvas(&self) -> gtk::DrawingArea {
        self.imp().canvas.get()
    }
    pub fn comparison_controls(&self) -> gtk::Box {
        self.imp().comparison_controls.get()
    }
    pub fn result_mode(&self) -> gtk::ToggleButton {
        self.imp().result_mode.get()
    }
    pub fn split_mode(&self) -> gtk::ToggleButton {
        self.imp().split_mode.get()
    }
    pub fn source_mode(&self) -> gtk::ToggleButton {
        self.imp().source_mode.get()
    }
    pub fn open_button(&self) -> gtk::Button {
        self.imp().open_button.get()
    }
    pub fn save_button(&self) -> gtk::Button {
        self.imp().save_button.get()
    }
    pub fn undo_button(&self) -> gtk::Button {
        self.imp().undo_button.get()
    }
    pub fn redo_button(&self) -> gtk::Button {
        self.imp().redo_button.get()
    }
    pub fn export_button(&self) -> gtk::Button {
        self.imp().export_button.get()
    }
    pub fn document_menu(&self) -> gtk::MenuButton {
        self.imp().document_menu.get()
    }
    pub fn sidebar_button(&self) -> gtk::ToggleButton {
        self.imp().sidebar_button.get()
    }
    pub fn file_spacer(&self) -> gtk::Separator {
        self.imp().file_spacer.get()
    }
    pub fn history_spacer(&self) -> gtk::Separator {
        self.imp().history_spacer.get()
    }
    pub fn welcome_open(&self) -> gtk::Button {
        self.imp().welcome_open.get()
    }
    pub fn welcome_project(&self) -> gtk::Button {
        self.imp().welcome_project.get()
    }
    pub fn welcome_example(&self) -> gtk::Button {
        self.imp().welcome_example.get()
    }
    pub fn status_bar(&self) -> gtk::Box {
        self.imp().status_bar.get()
    }
    pub fn status(&self) -> gtk::Label {
        self.imp().status.get()
    }
    pub fn pipeline_status(&self) -> gtk::Label {
        self.imp().pipeline_status.get()
    }
    pub fn progress(&self) -> gtk::ProgressBar {
        self.imp().progress.get()
    }
    pub fn cancel(&self) -> gtk::Button {
        self.imp().cancel.get()
    }
}

impl Default for ChromiatorMainShell {
    fn default() -> Self {
        Self::new()
    }
}
