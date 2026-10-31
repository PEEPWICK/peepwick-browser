//! Desktop notifications through the freedesktop notification service on the session bus,
//! for finished downloads and for sites allowed to notify.

use std::collections::HashMap;

use gtk::gio;
use gtk::glib;
use gtk::glib::prelude::*;

/// Shows a notification; nothing happens where there is no notification service.
pub fn show(summary: &str, body: &str) {
    let summary = summary.to_string();
    let body = body.to_string();
    gio::bus_get(
        gio::BusType::Session,
        None::<&gio::Cancellable>,
        move |bus| {
            let Ok(bus) = bus else { return };
            let hints: HashMap<String, glib::Variant> =
                HashMap::from([("desktop-entry".to_string(), "PEEPWICK Browser".to_variant())]);
            let parameters = (
                "PEEPWICK Browser",
                0u32,
                "peepwick-browser",
                summary.as_str(),
                body.as_str(),
                Vec::<String>::new(),
                hints,
                -1i32,
            )
                .to_variant();
            bus.call(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
                "Notify",
                Some(&parameters),
                None,
                gio::DBusCallFlags::NONE,
                5000,
                None::<&gio::Cancellable>,
                |result| {
                    if let Err(error) = result {
                        eprintln!("[notify] {error}");
                    }
                },
            );
        },
    );
}
