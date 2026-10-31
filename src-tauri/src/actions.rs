//! Actions shared by keyboard shortcuts, menus, the command bar and the UI.

use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use serde_json::json;
use webkit2gtk::{SaveMode, WebViewExt};

use crate::i18n::t;
use crate::settings::{self, BookmarksBar};
use crate::window::{self, BrowserWindow, OpenOptions};
use crate::{pages, session};

pub fn new_tab(win: &Rc<BrowserWindow>) {
    win.open_tab(OpenOptions {
        url: Some(pages::NEW_TAB_URL.into()),
        ..OpenOptions::default()
    });
    // The new tab page has its own search box; typing goes straight there.
    win.command(json!({ "type": "focus-page-search" }));
}

pub fn new_window(win: &BrowserWindow, private: bool, urls: Vec<String>) {
    if let Err(error) = window::open(&win.app_handle(), private, urls, None) {
        eprintln!("[window] could not open a window: {error}");
    }
}

/// Shows a built-in page: in the current tab when it already shows that page or a new tab,
/// in a tab that shows it already, or in a new tab.
pub fn open_page(win: &Rc<BrowserWindow>, page: &str, subpage: &str) {
    let Some(page) = pages::internal_page(&pages::internal_url(page, "")) else {
        return;
    };
    if let Some(active) = win.active_tab()
        && (active.page() == Some(page) || active.page() == Some("newtab"))
    {
        active.show_page(page, subpage);
        return;
    }
    if let Some(existing) = win
        .tabs_in_order(None)
        .into_iter()
        .find(|tab| tab.page() == Some(page))
    {
        existing.show_page(page, subpage);
        win.activate_tab(existing.id);
        return;
    }
    let tab = win.open_tab(OpenOptions {
        url: Some(pages::internal_url(page, "")),
        ..OpenOptions::default()
    });
    tab.show_page(page, subpage);
}

pub fn toggle_split(win: &Rc<BrowserWindow>) {
    if win.has_split() {
        win.unsplit();
        return;
    }
    if let (Some(active), Some(other)) = (win.active_tab(), win.previous_tab()) {
        win.split(other.id, active.id);
    }
}

/// Saves the page as one file (MHTML: the page and its images together).
pub fn save_page(win: &Rc<BrowserWindow>) {
    let Some(tab) = win.active_tab() else { return };
    let Some(view) = tab.view().filter(|_| tab.shows_web()) else {
        return;
    };
    let dialog = gtk::FileChooserNative::new(
        Some(&t("ctx.savePage")),
        win.gtk_window().as_ref(),
        gtk::FileChooserAction::Save,
        None,
        None,
    );
    dialog.set_do_overwrite_confirmation(true);
    let name: String = tab
        .display_title()
        .chars()
        .map(|c| if "/\\?%*:|\"<>".contains(c) { '-' } else { c })
        .take(120)
        .collect();
    dialog.set_current_name(&format!("{name}.mhtml"));
    if let Some(downloads) = glib::user_special_dir(glib::UserDirectory::Downloads) {
        let _ = dialog.set_current_folder(downloads);
    }
    dialog.connect_response(move |dialog, response| {
        if response == gtk::ResponseType::Accept
            && let Some(file) = dialog.file()
        {
            view.save_to_file(
                &file,
                SaveMode::Mhtml,
                None::<&gio::Cancellable>,
                |result| {
                    if let Err(error) = result {
                        eprintln!("[save] could not save the page: {error}");
                    }
                },
            );
        }
    });
    dialog.show();
}

pub fn is_available(win: &BrowserWindow, id: &str) -> bool {
    let active = win.active_tab();
    let web = active.as_ref().is_some_and(|tab| tab.shows_web());
    match id {
        "new-tab"
        | "new-window"
        | "new-incognito"
        | "settings"
        | "history"
        | "downloads"
        | "toggle-sidebar"
        | "bookmarks"
        | "toggle-bookmarks-bar"
        | "check-updates" => true,
        "split" => !win.has_split() && win.previous_tab().is_some(),
        "unsplit" => win.has_split(),
        "pin" => web,
        _ => false,
    }
}

pub fn run_command(win: &Rc<BrowserWindow>, id: &str) {
    match id {
        "new-tab" => new_tab(win),
        "new-window" => new_window(win, false, Vec::new()),
        "new-incognito" => new_window(win, true, Vec::new()),
        "settings" => open_page(win, "settings", ""),
        "history" => open_page(win, "history", ""),
        "downloads" => open_page(win, "downloads", ""),
        "bookmarks" => open_page(win, "bookmarks", ""),
        "check-updates" => open_page(win, "settings", "about"),
        "toggle-sidebar" => {
            settings::update(&json!({ "sidebarCollapsed": !settings::get().sidebar_collapsed }));
            window::settings_changed();
        }
        "toggle-bookmarks-bar" => {
            let mode = if settings::get().bookmarks_bar == BookmarksBar::Never {
                "always"
            } else {
                "never"
            };
            settings::update(&json!({ "bookmarksBar": mode }));
            window::settings_changed();
        }
        "split" | "unsplit" => toggle_split(win),
        "pin" => {
            if let Some(tab) = win.active_tab() {
                win.toggle_pin(tab.id);
            }
        }
        "quit" => session::quit(&win.app_handle()),
        _ => eprintln!("[actions] not ported yet: {id}"),
    }
}

/// The commands the command bar offers, in order.
pub const COMMANDS: [&str; 13] = [
    "new-tab",
    "new-window",
    "new-incognito",
    "split",
    "unsplit",
    "pin",
    "toggle-sidebar",
    "downloads",
    "history",
    "settings",
    "bookmarks",
    "toggle-bookmarks-bar",
    "check-updates",
];
