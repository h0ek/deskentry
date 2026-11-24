mod comparison;
mod fileio;
mod model;
mod scanner;
mod storage;
mod ui;
mod updates;
mod xdg;

use adw::prelude::*;
use gtk::gio;

const APP_ID: &str = "io.github.h0ek.DeskEntry";

fn main() -> gtk::glib::ExitCode {
    if let Err(error) = gio::resources_register_include!("deskentry.gresource") {
        eprintln!("Failed to register embedded resources: {error}");
        return gtk::glib::ExitCode::FAILURE;
    }

    let application = adw::Application::builder().application_id(APP_ID).build();
    application.connect_startup(|_| {
        if let Some(display) = gtk::gdk::Display::default() {
            let theme = gtk::IconTheme::for_display(&display);
            theme.add_resource_path("/io/github/h0ek/DeskEntry/icons/hicolor");
        }
    });
    application.connect_activate(ui::build_ui);
    application.run()
}
