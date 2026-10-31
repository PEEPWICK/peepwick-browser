//! The browser's menus: the page's right-click menu, the tab menu in the sidebar, the app
//! menu and the history page's menu. The UI draws them (`menu.rs`). Entries for features that haven't moved over from
//! Electron yet are left out until they have.

use std::rc::Rc;
use std::time::Duration;

use base64::Engine;
use gtk::gdk;
use gtk::gdk_pixbuf::PixbufLoader;
use gtk::gio;
use gtk::glib;
use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use javascriptcore::ValueExt;
use serde_json::json;
use webkit2gtk::{
    ContextMenu, ContextMenuAction, ContextMenuExt, ContextMenuItemExt, HitTestResult,
    HitTestResultExt, WebView, WebViewExt,
};

use crate::actions::{new_tab, new_window, open_page, run_command, save_page};
use crate::i18n::{Var, t, tv};
use crate::menu::{self, ClickEvent, MenuSpec, item, separator, submenu};
use crate::search::search_url;
use crate::tab::Tab;
use crate::window;
use crate::window::{BrowserWindow, OpenOptions, USER_AGENT};
use crate::{history, session, settings};

pub(crate) fn on_window(
    win: &Rc<BrowserWindow>,
    action: impl Fn(&Rc<BrowserWindow>) + 'static,
) -> impl Fn(ClickEvent) + 'static {
    let win = win.clone();
    move |_| action(&win)
}

/// Acts on a tab looked up when chosen: it may have closed while the menu was open.
fn on_tab(
    win: &Rc<BrowserWindow>,
    id: u32,
    action: impl Fn(&Rc<Tab>) + 'static,
) -> impl Fn(ClickEvent) + 'static {
    let win = win.clone();
    move |_| {
        if let Some(tab) = win.tab(id) {
            action(&tab);
        }
    }
}

fn on_active(
    win: &Rc<BrowserWindow>,
    action: impl Fn(&Rc<Tab>) + 'static,
) -> impl Fn(ClickEvent) + 'static {
    let win = win.clone();
    move |_| {
        if let Some(tab) = win.active_tab() {
            action(&tab);
        }
    }
}

pub fn copy_text(text: &str) {
    gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD).set_text(text);
}

// ── Page ──────────────────────────────────────────────────────────────────────

/// What was right-clicked.
struct Hit {
    link: Option<String>,
    link_label: String,
    image: Option<String>,
    media: Option<String>,
    editable: bool,
}

pub fn page_menu(
    tab: &Rc<Tab>,
    view: &WebView,
    proposed: &ContextMenu,
    result: &HitTestResult,
    x: f64,
    y: f64,
) {
    let Some(win) = tab.window() else { return };
    // The click is in the view's coordinates; the UI draws the menu over the whole window.
    let (x, y) = view
        .translate_coordinates(&win.ui_view(), x as i32, y as i32)
        .unwrap_or((x as i32, y as i32));
    let uri = |value: Option<glib::GString>| value.map(|v| v.to_string()).filter(|v| !v.is_empty());
    let hit = Hit {
        link_label: result
            .link_label()
            .map(|l| l.trim().to_string())
            .unwrap_or_default(),
        link: if result.context_is_link() {
            uri(result.link_uri())
        } else {
            None
        },
        image: if result.context_is_image() {
            uri(result.image_uri())
        } else {
            None
        },
        media: if result.context_is_media() {
            uri(result.media_uri())
        } else {
            None
        },
        editable: result.context_is_editable(),
    };
    let guesses = spelling_guesses(proposed);
    if result.context_is_selection() && !hit.editable {
        let (tab, page) = (tab.clone(), view.clone());
        view.evaluate_javascript(
            "window.getSelection().toString()",
            None,
            None,
            None::<&gio::Cancellable>,
            move |text| {
                let selection = text
                    .map(|v| v.to_str().trim().to_string())
                    .unwrap_or_default();
                show_page_menu(&win, &tab, &page, hit, guesses, selection, x, y);
            },
        );
    } else {
        show_page_menu(&win, tab, view, hit, guesses, String::new(), x, y);
    }
}

/// WebKit puts spelling guesses in its own menu; their labels are only on the older GtkAction.
fn spelling_guesses(proposed: &ContextMenu) -> Vec<String> {
    proposed
        .items()
        .iter()
        .filter(|item| item.stock_action() == ContextMenuAction::SpellingGuess)
        .filter_map(|item| unsafe {
            let action =
                webkit2gtk::ffi::webkit_context_menu_item_get_action(item.to_glib_none().0);
            if action.is_null() {
                return None;
            }
            let label = gtk::ffi::gtk_action_get_label(action.cast());
            (!label.is_null()).then(|| {
                std::ffi::CStr::from_ptr(label)
                    .to_string_lossy()
                    .into_owned()
            })
        })
        .take(5)
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn show_page_menu(
    win: &Rc<BrowserWindow>,
    tab: &Rc<Tab>,
    view: &WebView,
    hit: Hit,
    guesses: Vec<String>,
    selection: String,
    x: i32,
    y: i32,
) {
    let id = tab.id;
    let has_selection = !selection.is_empty();
    let mut items: Vec<MenuSpec> = Vec::new();

    for guess in guesses {
        let page = view.clone();
        // WebKit has selected the misspelled word; typing the guess replaces it.
        let script = format!(
            "document.execCommand('insertText', false, {})",
            json!(guess)
        );
        items.push(item(guess, move |_| {
            page.evaluate_javascript(&script, None, None, None::<&gio::Cancellable>, |_| {});
        }));
    }
    items.push(separator());

    let opens = |url: &str| {
        ["http:", "https:", "ftp:", "file:", "mailto:"]
            .iter()
            .any(|s| url.starts_with(s))
    };
    if let Some(link) = hit.link.clone().filter(|l| opens(l)) {
        let url = link.clone();
        items.push(item(
            t("ctx.openLinkNewTab"),
            on_window(win, move |win| {
                win.open_tab(OpenOptions {
                    url: Some(url.clone()),
                    background: true,
                    opener: Some(id),
                    ..OpenOptions::default()
                });
            }),
        ));
        let url = link.clone();
        items.push(item(
            t("ctx.openLinkSplit"),
            on_window(win, move |win| {
                let other = win.open_tab(OpenOptions {
                    url: Some(url.clone()),
                    background: true,
                    opener: Some(id),
                    ..OpenOptions::default()
                });
                win.split(id, other.id);
            }),
        ));
        let url = link.clone();
        items.push(item(
            t("ctx.openLinkIncognito"),
            on_window(win, move |win| new_window(win, true, vec![url.clone()])),
        ));
        let bookmarkable =
            !win.incognito && crate::bookmarks::bookmarkable(&link) && !link.starts_with("mailto:");
        let (url, title) = (link.clone(), hit.link_label.clone());
        items.push(item(t("ctx.copyLink"), move |_| copy_text(&link)));
        if bookmarkable {
            items.push(item(t("ctx.bookmarkLink"), move |_| {
                crate::bookmarks::create("bar", None, &title, Some(&url));
            }));
        }
        items.push(separator());
    }

    if let Some(src) = hit.image.clone() {
        let url = src.clone();
        items.push(item(
            t("ctx.openImage"),
            on_window(win, move |win| {
                win.open_tab(OpenOptions {
                    url: Some(url.clone()),
                    background: true,
                    opener: Some(id),
                    ..OpenOptions::default()
                });
            }),
        ));
        let (page, url) = (view.clone(), src.clone());
        items.push(item(t("ctx.saveImage"), move |_| {
            page.download_uri(&url);
        }));
        let url = src.clone();
        items.push(item(t("ctx.copyImage"), move |_| copy_image(url.clone())));
        items.push(item(t("ctx.copyImageAddress"), move |_| copy_text(&src)));
        items.push(separator());
    } else if let Some(src) = hit
        .media
        .clone()
        .filter(|s| s.starts_with("http:") || s.starts_with("https:"))
    {
        // blob: and MediaSource videos (most streaming sites) can't be saved as a file.
        let page = view.clone();
        items.push(item(t("ctx.saveMedia"), move |_| {
            page.download_uri(&src);
        }));
        items.push(separator());
    }

    if hit.editable {
        for (key, command, keys) in [
            ("ctx.cut", "Cut", "Ctrl+X"),
            ("ctx.copy", "Copy", "Ctrl+C"),
            ("ctx.paste", "Paste", "Ctrl+V"),
            ("ctx.selectAll", "SelectAll", "Ctrl+A"),
        ] {
            let page = view.clone();
            items.push(
                item(t(key), move |_| page.execute_editing_command(command)).accelerator(keys),
            );
        }
        items.push(separator());
    } else if has_selection {
        let page = view.clone();
        items.push(
            item(t("ctx.copy"), move |_| page.execute_editing_command("Copy"))
                .accelerator("Ctrl+C"),
        );
        let short: String = if selection.chars().count() > 24 {
            format!("{}…", selection.chars().take(24).collect::<String>())
        } else {
            selection.clone()
        };
        items.push(item(
            tv("ctx.searchFor", &[("text", Var::Text(&short))]),
            on_window(win, move |win| {
                let url = search_url(settings::get().search_engine, &selection);
                win.open_tab(OpenOptions {
                    url: Some(url),
                    opener: Some(id),
                    ..OpenOptions::default()
                });
            }),
        ));
        items.push(separator());
    }

    if hit.link.is_none()
        && hit.image.is_none()
        && hit.media.is_none()
        && !hit.editable
        && !has_selection
    {
        items.push(
            item(t("ctx.back"), on_tab(win, id, |tab| tab.back()))
                .accelerator("Alt+Left")
                .enabled(tab.can_go_back()),
        );
        items.push(
            item(t("ctx.forward"), on_tab(win, id, |tab| tab.forward()))
                .accelerator("Alt+Right")
                .enabled(tab.can_go_forward()),
        );
        items.push(
            item(t("ctx.reload"), on_tab(win, id, |tab| tab.reload(false))).accelerator("Ctrl+R"),
        );
        items.push(separator());
        items.push(item(t("ctx.savePage"), on_window(win, save_page)).accelerator("Ctrl+S"));
        items.push(item(t("ctx.print"), on_tab(win, id, |tab| tab.print())).accelerator("Ctrl+P"));
        items.push(separator());
    }

    items.push(item(
        t("ctx.inspect"),
        on_tab(win, id, |tab| tab.devtools()),
    ));
    menu::popup(win, items, f64::from(x), f64::from(y), false, None);
}

/// WebKit's own "copy image" only works from its native menu, so the image is fetched again
/// (data: images are decoded in place) and put on the clipboard.
fn copy_image(src: String) {
    std::thread::spawn(move || {
        let Some(bytes) = image_bytes(&src) else {
            return;
        };
        glib::idle_add_once(move || {
            let loader = PixbufLoader::new();
            if loader.write(&bytes).is_ok()
                && loader.close().is_ok()
                && let Some(pixbuf) = loader.pixbuf()
            {
                gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD).set_image(&pixbuf);
            }
        });
    });
}

fn image_bytes(src: &str) -> Option<Vec<u8>> {
    if let Some(data) = src.strip_prefix("data:") {
        let (meta, payload) = data.split_once(',')?;
        return if meta.ends_with(";base64") {
            base64::engine::general_purpose::STANDARD
                .decode(payload.trim())
                .ok()
        } else {
            Some(percent_encoding::percent_decode_str(payload).collect())
        };
    }
    if !(src.starts_with("http:") || src.starts_with("https:")) {
        return None;
    }
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .build()
        .ok()?;
    let bytes = client
        .get(src)
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .bytes()
        .ok()?;
    (bytes.len() <= 50 << 20).then(|| bytes.to_vec())
}

// ── Tab ───────────────────────────────────────────────────────────────────────

pub fn tab_menu(win: &Rc<BrowserWindow>, id: u32, x: f64, y: f64) {
    let (Some(tab), Some(workspace_id)) = (win.tab(id), win.workspace_of(id)) else {
        return;
    };
    let tabs = win.tabs_in_order(Some(&workspace_id));
    let index = tabs.iter().position(|t| t.id == id);
    let in_split = win.in_split(id);
    let internal = tab.page().is_some();
    let mut items = vec![
        item(
            t("tabmenu.reload"),
            on_tab(win, id, |tab| tab.reload(false)),
        )
        .enabled(!internal),
        item(
            t("tabmenu.duplicate"),
            on_window(win, move |win| win.duplicate_tab(id)),
        )
        .enabled(!internal),
        item(
            t(if tab.pinned.get() {
                "tabmenu.unpin"
            } else {
                "tabmenu.pin"
            }),
            on_window(win, move |win| win.toggle_pin(id)),
        ),
        item(
            t(if tab.is_muted() {
                "tabmenu.unmute"
            } else {
                "tabmenu.mute"
            }),
            on_tab(win, id, |tab| tab.set_muted(!tab.is_muted())),
        ),
    ];
    if !win.incognito
        && !internal
        && crate::bookmarks::bookmarkable(&tab.url())
        && !tab.url().starts_with("ftp:")
    {
        items.push(item(
            t("tabmenu.bookmark"),
            on_tab(win, id, |tab| {
                crate::bookmarks::create("bar", None, &tab.display_title(), Some(&tab.url()));
            }),
        ));
    }
    items.push(separator());
    if let Some(active) = win
        .active_tab()
        .map(|t| t.id)
        .filter(|a| *a != id && !in_split)
    {
        items.push(item(
            t("tabmenu.split"),
            on_window(win, move |win| win.split(active, id)),
        ));
    }
    if in_split {
        items.push(item(
            t("tabmenu.unsplit"),
            on_window(win, |win| win.unsplit()),
        ));
    }
    let targets: Vec<MenuSpec> = win
        .workspace_list()
        .into_iter()
        .filter(|(other, ..)| *other != workspace_id)
        .map(|(other, name, count)| {
            item(
                name,
                on_window(win, move |win| win.move_tab(id, &other, count, None)),
            )
        })
        .collect();
    if !targets.is_empty() {
        items.push(submenu(t("tabmenu.moveTo"), targets));
    }
    items.push(
        item(
            t("tabmenu.sleep"),
            on_window(win, move |win| win.sleep_tab(id)),
        )
        .enabled(tab.view().is_some() && !win.is_shown(id)),
    );
    let url = tab.display_url();
    items.push(item(t("tabmenu.copyLink"), move |_| copy_text(&url)).enabled(!internal));
    items.push(separator());
    let others: Vec<u32> = tabs
        .iter()
        .filter(|t| t.id != id && !t.pinned.get())
        .map(|t| t.id)
        .collect();
    items.push(
        item(
            t("tabmenu.closeOthers"),
            on_window(win, move |win| {
                for other in &others {
                    win.close_tab(*other, true);
                }
            }),
        )
        .enabled(tabs.len() > 1),
    );
    let below: Vec<u32> = index
        .map(|i| {
            tabs[i + 1..]
                .iter()
                .filter(|t| !t.pinned.get())
                .map(|t| t.id)
                .collect()
        })
        .unwrap_or_default();
    let has_below = index.is_some_and(|i| i + 1 < tabs.len());
    items.push(
        item(
            t("tabmenu.closeBelow"),
            on_window(win, move |win| {
                for other in &below {
                    win.close_tab(*other, true);
                }
            }),
        )
        .enabled(has_below),
    );
    items.push(item(
        t("tabmenu.close"),
        on_window(win, move |win| win.close_tab(id, true)),
    ));
    menu::popup(win, items, x, y, false, None);
}

// ── App ───────────────────────────────────────────────────────────────────────

fn bookmarks_submenu(win: &Rc<BrowserWindow>) -> MenuSpec {
    let available = |id: &str| crate::actions::is_available(win, id);
    let command = |id: &'static str| on_window(win, move |win| run_command(win, id));
    submenu(
        t("menu.bookmarks"),
        vec![
            item(t("cmd.bookmark-page"), command("bookmark-page"))
                .accelerator("Ctrl+D")
                .enabled(available("bookmark-page")),
            item(t("cmd.bookmark-tabs"), command("bookmark-tabs"))
                .accelerator("Ctrl+Shift+D")
                .enabled(available("bookmark-tabs")),
            separator(),
            item(t("bookmarks.showBar"), command("toggle-bookmarks-bar"))
                .checkbox(
                    crate::settings::get().bookmarks_bar != crate::settings::BookmarksBar::Never,
                )
                .accelerator("Ctrl+Shift+B"),
            item(
                t("bookmarks.manager"),
                on_window(win, |win| open_page(win, "bookmarks", "")),
            )
            .accelerator("Ctrl+Shift+O"),
            separator(),
            item(t("import.menu"), command("import-bookmarks")),
            item(
                t("bookmarks.exportMenu"),
                on_window(win, |win| {
                    crate::bookmarks::export(win);
                }),
            ),
        ],
    )
}

/// `x` is the menu button's right edge: the menu lines up with it.
pub fn app_menu(win: &Rc<BrowserWindow>, x: f64, y: f64) {
    let web = win
        .active_tab()
        .is_some_and(|tab| tab.shows_web() && tab.view().is_some());
    let mut items = vec![
        item(t("menu.newTab"), on_window(win, new_tab)).accelerator("Ctrl+T"),
        item(
            t("menu.newWindow"),
            on_window(win, |win| new_window(win, false, Vec::new())),
        )
        .accelerator("Ctrl+N"),
        item(
            t("menu.newIncognito"),
            on_window(win, |win| new_window(win, true, Vec::new())),
        )
        .accelerator("Ctrl+Shift+N"),
        separator(),
        item(
            t("menu.history"),
            on_window(win, |win| open_page(win, "history", "")),
        )
        .accelerator("Ctrl+H"),
    ];
    if !win.incognito {
        items.push(bookmarks_submenu(win));
    }
    items.extend([
        item(
            t("menu.downloads"),
            on_window(win, |win| open_page(win, "downloads", "")),
        )
        .accelerator("Ctrl+J"),
        separator(),
        item(t("menu.zoomIn"), on_active(win, |tab| tab.zoom("in")))
            .accelerator("Ctrl+=")
            .enabled(web),
        item(t("menu.zoomOut"), on_active(win, |tab| tab.zoom("out")))
            .accelerator("Ctrl+-")
            .enabled(web),
        item(t("menu.zoomReset"), on_active(win, |tab| tab.zoom("reset")))
            .accelerator("Ctrl+0")
            .enabled(web),
        separator(),
        item(
            t("menu.find"),
            on_window(win, |win| win.command(json!({ "type": "find" }))),
        )
        .accelerator("Ctrl+F")
        .enabled(web),
        item(t("menu.print"), on_active(win, |tab| tab.print()))
            .accelerator("Ctrl+P")
            .enabled(web),
        item(
            t("menu.fullscreen"),
            on_window(win, |win| win.toggle_fullscreen()),
        )
        .accelerator("F11"),
        item(t("menu.devtools"), on_active(win, |tab| tab.devtools()))
            .accelerator("F12")
            .enabled(web),
        separator(),
        item(
            t("menu.settings"),
            on_window(win, |win| open_page(win, "settings", "")),
        ),
        item(
            t("menu.about"),
            on_window(win, |win| run_command(win, "check-updates")),
        ),
        separator(),
        item(
            t("menu.quit"),
            on_window(win, |win| session::quit(&win.app_handle())),
        )
        .accelerator("Ctrl+Q"),
    ]);
    menu::popup(win, items, x, y, true, None);
}

// ── History ───────────────────────────────────────────────────────────────────

/// Lets every window's history page know the list changed under it.
pub fn history_changed() {
    for win in window::all() {
        win.emit("command", json!({ "type": "history-changed" }));
    }
}

pub fn history_menu(win: &Rc<BrowserWindow>, id: i64, url: &str, x: f64, y: f64) {
    if !(url.starts_with("http:") || url.starts_with("https:")) {
        return;
    }
    let host = crate::pages::host_of(url);
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let (a, b, c, d) = (
        url.to_string(),
        url.to_string(),
        url.to_string(),
        url.to_string(),
    );
    let site = host.clone();
    let items = vec![
        item(
            t("bookmarks.openNewTab"),
            on_window(win, move |win| {
                win.open_tab(OpenOptions {
                    url: Some(a.clone()),
                    background: true,
                    ..OpenOptions::default()
                });
            }),
        ),
        item(
            t("bookmarks.openNewWindow"),
            on_window(win, move |win| new_window(win, false, vec![b.clone()])),
        ),
        item(
            t("bookmarks.openIncognito"),
            on_window(win, move |win| new_window(win, true, vec![c.clone()])),
        ),
        item(t("ctx.copyLink"), move |_| copy_text(&d)),
        separator(),
        item(
            t("history.moreFromSite"),
            on_window(win, move |win| {
                win.emit("command", json!({ "type": "history-search", "text": site }))
            }),
        ),
        separator(),
        item(t("history.remove"), move |_| {
            history::remove_visits(&[id]);
            history_changed();
        }),
        item(
            t("history.removeSite"),
            on_window(win, move |win| {
                let question = menu::Question {
                    title: tv("history.removeSiteConfirm", &[("site", Var::Text(&host))]),
                    body: Some(t("history.removeSiteBody")),
                    confirm: t("history.removeSiteRun"),
                    cancel: t("common.cancel"),
                    danger: true,
                };
                let host = host.clone();
                menu::confirm(win, question, move |ok| {
                    if ok {
                        history::delete_host(&host);
                        history_changed();
                    }
                });
            }),
        ),
    ];
    menu::popup(win, items, x, y, false, None);
}
