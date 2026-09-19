//! Modal GTK4 confirmations with stable response identifiers and no document authority.
//!
//! Closing, Escape, parent destruction, and a dropped waiting future never accept an action.
//! Custom windows retain destructive-button styling without deprecated GtkDialog APIs.

use std::cell::RefCell;
use std::future::poll_fn;
use std::rc::Rc;
use std::task::{Poll, Waker};

use gtk::prelude::*;

/// Visual emphasis for an explicitly destructive response.
#[derive(Clone, Copy)]
pub enum ResponseAppearance {
    Destructive,
}

/// Presentation-only builder for a confirmation's heading and explanatory body.
pub struct AlertDialogBuilder {
    heading: String,
    body: String,
}

/// Reusable prompt description; each presentation creates its own response lifetime.
pub struct AlertDialog {
    heading: String,
    body: String,
    responses: RefCell<Vec<(String, String, bool)>>,
}

/// Single-assignment completion, shared only on GTK's main thread.
#[derive(Default)]
struct Completion {
    completed: bool,
    response: Option<String>,
    waker: Option<Waker>,
}

impl Completion {
    /// Returns the waiter to wake after releasing the interior mutable borrow.
    fn finish(&mut self, response: String) -> Option<Waker> {
        if self.completed {
            return None;
        }
        self.completed = true;
        self.response = Some(response);
        self.waker.take()
    }
}

fn complete(completion: &RefCell<Completion>, response: &str) {
    let waker = completion.borrow_mut().finish(response.to_owned());
    if let Some(waker) = waker {
        waker.wake();
    }
}

/// Dropping an abandoned future must not strand a modal window above its parent.
struct WaitingWindow(gtk::Window);

impl Drop for WaitingWindow {
    fn drop(&mut self) {
        self.0.destroy();
    }
}

impl AlertDialog {
    /// Begins a prompt description without allocating GTK widgets.
    pub fn builder() -> AlertDialogBuilder {
        AlertDialogBuilder {
            heading: String::new(),
            body: String::new(),
        }
    }

    /// Appends a displayed button with a stable application-facing response ID.
    pub fn add_response(&self, id: &str, label: &str) {
        self.responses
            .borrow_mut()
            .push((id.into(), label.into(), false));
    }

    /// Marks an existing response as destructive without changing its meaning.
    pub fn set_response_appearance(&self, id: &str, _: ResponseAppearance) {
        if let Some(response) = self
            .responses
            .borrow_mut()
            .iter_mut()
            .find(|response| response.0 == id)
        {
            response.2 = true;
        }
    }

    /// Resolves once to a button ID, or `cancel` on dismissal or parent destruction.
    pub async fn choose_future(&self, parent: Option<&impl IsA<gtk::Window>>) -> String {
        let completion = Rc::new(RefCell::new(Completion::default()));
        let window = WaitingWindow(self.create(parent, &completion));
        window.0.present();
        poll_fn(|context| {
            let mut completion = completion.borrow_mut();
            if let Some(response) = completion.response.take() {
                Poll::Ready(response)
            } else {
                completion.waker = Some(context.waker().clone());
                Poll::Pending
            }
        })
        .await
    }

    /// Shows an informational prompt; GTK retains the window until it is dismissed.
    pub fn present(&self, parent: Option<&impl IsA<gtk::Window>>) {
        let completion = Rc::new(RefCell::new(Completion::default()));
        self.create(parent, &completion).present();
    }

    fn create(
        &self,
        parent: Option<&impl IsA<gtk::Window>>,
        completion: &Rc<RefCell<Completion>>,
    ) -> gtk::Window {
        let window = gtk::Window::builder()
            .modal(true)
            .title(&self.heading)
            .destroy_with_parent(true)
            .resizable(false)
            .default_width(420)
            .accessible_role(gtk::AccessibleRole::Dialog)
            .build();
        if let Some(parent) = parent {
            window.set_transient_for(Some(parent));
        }
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(16)
            .margin_top(16)
            .margin_bottom(16)
            .margin_start(18)
            .margin_end(18)
            .build();
        let label = gtk::Label::builder()
            .label(&self.body)
            .wrap(true)
            .max_width_chars(64)
            .xalign(0.0)
            .build();
        content.append(&label);
        let actions = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .halign(gtk::Align::End)
            .build();
        let mut responses = self.responses.borrow().clone();
        if responses.is_empty() {
            responses.push(("cancel".into(), "Close".into(), false));
        }
        let default_index = responses
            .iter()
            .position(|response| response.0 == "cancel")
            .or_else(|| responses.iter().position(|response| !response.2));
        for (index, (id, label, destructive)) in responses.into_iter().enumerate() {
            let button = gtk::Button::with_label(&label);
            if destructive {
                button.add_css_class("destructive-action");
            }
            actions.append(&button);
            if Some(index) == default_index {
                window.set_default_widget(Some(&button));
                button.grab_focus();
            }
            let completion = completion.clone();
            let weak_window = window.downgrade();
            button.connect_clicked(move |_| {
                complete(&completion, &id);
                if let Some(window) = weak_window.upgrade() {
                    window.close();
                }
            });
        }
        content.append(&actions);
        window.set_child(Some(&content));
        let closed = completion.clone();
        window.connect_close_request(move |_| {
            complete(&closed, "cancel");
            gtk::glib::Propagation::Proceed
        });
        let destroyed = completion.clone();
        window.connect_destroy(move |_| complete(&destroyed, "cancel"));
        let keys = gtk::EventControllerKey::new();
        let weak_window = window.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(window) = weak_window.upgrade() {
                    window.close();
                }
                gtk::glib::Propagation::Stop
            } else {
                gtk::glib::Propagation::Proceed
            }
        });
        window.add_controller(keys);
        window
    }
}

impl AlertDialogBuilder {
    /// Sets the native window heading.
    pub fn heading(mut self, heading: impl Into<String>) -> Self {
        self.heading = heading.into();
        self
    }

    /// Sets the wrapped explanatory text.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// Finishes a description with no predefined responses.
    pub fn build(self) -> AlertDialog {
        AlertDialog {
            heading: self.heading,
            body: self.body,
            responses: RefCell::new(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_after_button_activation_cannot_replace_the_response() {
        let completion = RefCell::new(Completion::default());
        complete(&completion, "delete");
        complete(&completion, "cancel");
        assert_eq!(
            completion.borrow_mut().response.take().as_deref(),
            Some("delete")
        );
        complete(&completion, "cancel");
        assert!(completion.borrow().response.is_none());
    }

    #[test]
    fn cancellation_is_terminal_and_wakes_the_waiter_once() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use std::task::Wake;
        struct Counter(AtomicUsize);
        impl Wake for Counter {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let counter = Arc::new(Counter(AtomicUsize::new(0)));
        let completion = RefCell::new(Completion {
            waker: Some(Waker::from(counter.clone())),
            ..Completion::default()
        });
        complete(&completion, "cancel");
        complete(&completion, "delete");
        assert_eq!(counter.0.load(Ordering::Relaxed), 1);
        assert_eq!(completion.borrow().response.as_deref(), Some("cancel"));
    }
}
