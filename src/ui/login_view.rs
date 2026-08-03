//! Login screen: OAuth device-flow code display + status.
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, Label};

use crate::state::WorkerCommand;
use crate::worker::Worker;

#[derive(Clone)]
pub struct LoginView {
    pub root: GtkBox,
    code_label: Label,
    url_label: Label,
    status_label: Label,
    sign_in_button: Button,
    cancel_button: Button,
}

impl LoginView {
    pub fn new(worker: Worker) -> Self {
        let root = GtkBox::new(gtk4::Orientation::Vertical, 12);
        root.set_valign(Align::Center);
        root.set_halign(Align::Center);
        root.add_css_class("login");

        let title = Label::new(Some("Yandex Music"));
        title.add_css_class("title-1");
        let subtitle = Label::new(Some("Sign in to start listening"));
        subtitle.add_css_class("dim-label");

        let code_label = Label::new(None);
        code_label.add_css_class("user-code");
        code_label.set_visible(false);

        let url_label = Label::new(None);
        url_label.set_selectable(true);
        url_label.set_wrap(true);
        url_label.set_max_width_chars(60);
        url_label.set_visible(false);

        let status_label = Label::new(Some(""));
        status_label.set_wrap(true);
        status_label.set_max_width_chars(60);
        status_label.add_css_class("dim-label");

        let sign_in_button = Button::with_label("Sign in");
        sign_in_button.add_css_class("suggested-action");
        let cancel_button = Button::with_label("Cancel");
        cancel_button.set_visible(false);

        let buttons = GtkBox::new(gtk4::Orientation::Horizontal, 8);
        buttons.set_halign(Align::Center);
        buttons.append(&sign_in_button);
        buttons.append(&cancel_button);

        root.append(&title);
        root.append(&subtitle);
        root.append(&code_label);
        root.append(&url_label);
        root.append(&status_label);
        root.append(&buttons);

        sign_in_button.connect_clicked({
            let worker = worker.clone();
            move |_| worker.send(WorkerCommand::BeginLogin)
        });
        cancel_button.connect_clicked(move |_| worker.send(WorkerCommand::CancelLogin));

        Self {
            root,
            code_label,
            url_label,
            status_label,
            sign_in_button,
            cancel_button,
        }
    }

    /// Show the device code and where to enter it.
    pub fn show_code(&self, user_code: &str, verification_url: &str) {
        let escaped_url = glib::markup_escape_text(verification_url);
        self.code_label
            .set_markup(&format!("<big><b>{user_code}</b></big>"));
        self.url_label.set_markup(&format!(
            "Open <a href=\"{escaped_url}\">{escaped_url}</a> and enter this code"
        ));
        self.url_label.set_visible(true);
        self.code_label.set_visible(true);
        self.sign_in_button.set_visible(false);
        self.cancel_button.set_visible(true);
        self.status_label.set_text("Waiting for confirmation…");
    }

    /// Show an error and re-enable the sign-in button.
    pub fn set_error(&self, message: &str) {
        let escaped = glib::markup_escape_text(message);
        self.status_label
            .set_markup(&format!("<span color=\"#ff5b5b\">{escaped}</span>"));
        self.sign_in_button.set_visible(true);
        self.cancel_button.set_visible(false);
    }

    /// Reset to the idle state (after a successful login).
    pub fn reset(&self) {
        self.code_label.set_visible(false);
        self.url_label.set_visible(false);
        self.status_label.set_text("");
        self.sign_in_button.set_visible(true);
        self.cancel_button.set_visible(false);
    }
}
