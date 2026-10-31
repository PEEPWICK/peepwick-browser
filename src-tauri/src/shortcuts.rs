//! Keyboard shortcuts. Every tab view and the UI view hand their key presses here first, so
//! shortcuts work wherever keyboard focus is. Letters are read from the keyboard's first
//! layout, so Ctrl+T still works while a Cyrillic or Greek layout is active.

use std::rc::Rc;

use gtk::gdk;
use gtk::gdk::keys::constants as key;
use serde_json::json;

use crate::actions::{new_tab, new_window, open_page, run_command, save_page, toggle_split};
use crate::session;
use crate::window::BrowserWindow;

/// The Latin letter under the pressed key, whatever layout is active.
fn letter(event: &gdk::EventKey) -> Option<char> {
    let display = gdk::Display::default()?;
    let keymap = gdk::Keymap::for_display(&display)?;
    let (keyval, ..) = keymap.translate_keyboard_state(
        u32::from(event.hardware_keycode()),
        gdk::ModifierType::empty(),
        0,
    )?;
    let c = gdk::keys::Key::from(keyval).to_lower().to_unicode()?;
    c.is_ascii_lowercase().then_some(c)
}

/// 0–9 from the number row or the keypad.
fn digit(event: &gdk::EventKey) -> Option<u32> {
    let keyval = event.keyval();
    for (i, k) in [
        key::KP_0,
        key::KP_1,
        key::KP_2,
        key::KP_3,
        key::KP_4,
        key::KP_5,
        key::KP_6,
        key::KP_7,
        key::KP_8,
        key::KP_9,
    ]
    .into_iter()
    .enumerate()
    {
        if keyval == k {
            return Some(i as u32);
        }
    }
    // The number row is keycodes 10–19 (1…9, 0) on every layout.
    match event.hardware_keycode() {
        code @ 10..=18 => Some(u32::from(code) - 9),
        19 => Some(0),
        _ => None,
    }
}

pub fn handle(win: &Rc<BrowserWindow>, event: &gdk::EventKey) -> bool {
    let state = event.state();
    let ctrl = state.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::META_MASK);
    let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
    let alt = state.contains(gdk::ModifierType::MOD1_MASK);
    let keyval = event.keyval();
    let tab = win.active_tab();
    let l = letter(event);
    let n = digit(event);

    // ── Without Ctrl ──
    if !ctrl {
        if alt && !shift && keyval == key::Left || keyval == key::Back {
            if let Some(tab) = tab {
                tab.back();
            }
            return true;
        }
        if alt && !shift && keyval == key::Right || keyval == key::Forward {
            if let Some(tab) = tab {
                tab.forward();
            }
            return true;
        }
        if alt && !shift {
            if let Some(n) = n.filter(|n| *n > 0) {
                if let Some((id, ..)) = win.workspace_list().get(n as usize - 1) {
                    win.activate_workspace(id);
                }
                return true;
            }
            if l == Some('d') {
                win.command(json!({ "type": "focus-omnibox" }));
                return true;
            }
        }
        if alt {
            return false;
        }
        if keyval == key::F5 {
            if let Some(tab) = tab {
                tab.reload(shift);
            }
        } else if keyval == key::F6 {
            win.command(json!({ "type": "focus-omnibox" }));
        } else if keyval == key::F3 {
            win.command(json!({ "type": "find" }));
            if let Some(tab) = tab
                && let Some(text) = tab.find_text()
            {
                tab.find_in_page(&text, !shift, true);
            }
        } else if keyval == key::F11 {
            win.toggle_fullscreen();
        } else if keyval == key::F12 {
            if let Some(tab) = tab {
                tab.devtools();
            }
        } else if keyval == key::Escape {
            if shift {
                win.command(json!({ "type": "task-manager" }));
                return true;
            }
            match tab {
                Some(tab) if tab.is_finding() => {
                    tab.stop_find();
                    win.emit("command", json!({ "type": "escape" }));
                    return true;
                }
                Some(tab) if tab.is_loading() => {
                    tab.stop();
                    return false;
                }
                _ => return false,
            }
        } else {
            return false;
        }
        return true;
    }

    // ── With Ctrl ──
    if alt {
        return false;
    }

    if let Some(n) = n
        && !shift
    {
        if n == 0 {
            if let Some(tab) = tab {
                tab.zoom("reset");
            }
            return true;
        }
        let tabs = win.tabs_in_order(None);
        let target = if n == 9 {
            tabs.last()
        } else {
            tabs.get(n as usize - 1)
        };
        if let Some(target) = target {
            win.activate_tab(target.id);
        }
        return true;
    }

    if keyval == key::plus || keyval == key::equal || keyval == key::KP_Add {
        if let Some(tab) = tab {
            tab.zoom("in");
        }
        return true;
    }
    if keyval == key::minus || keyval == key::underscore || keyval == key::KP_Subtract {
        if let Some(tab) = tab {
            tab.zoom("out");
        }
        return true;
    }

    if keyval == key::Tab
        || keyval == key::ISO_Left_Tab
        || keyval == key::Page_Up
        || keyval == key::Page_Down
    {
        let tabs = win.tabs_in_order(None);
        let Some(tab) = tab else { return true };
        if tabs.len() < 2 {
            return true;
        }
        let back =
            keyval == key::Page_Up || keyval == key::ISO_Left_Tab || (keyval == key::Tab && shift);
        let index = tabs.iter().position(|t| t.id == tab.id).unwrap_or(0);
        let next = (index + if back { tabs.len() - 1 } else { 1 }) % tabs.len();
        win.activate_tab(tabs[next].id);
        return true;
    }

    if keyval == key::F4 {
        if let Some(tab) = tab {
            win.close_tab(tab.id, true);
        }
        return true;
    }
    if keyval == key::Delete && shift {
        win.command(json!({ "type": "clear-data" }));
        return true;
    }
    if keyval == key::comma {
        open_page(win, "settings", "");
        return true;
    }

    let Some(l) = l else { return false };
    if shift {
        match l {
            't' => win.reopen_closed_tab(),
            'n' => new_window(win, true, Vec::new()),
            'w' => win.window_control("close"),
            'r' => {
                if let Some(tab) = tab {
                    tab.reload(true);
                }
            }
            'g' => {
                if let Some(tab) = tab
                    && let Some(text) = tab.find_text()
                {
                    tab.find_in_page(&text, false, true);
                }
            }
            'i' | 'j' => {
                if let Some(tab) = tab {
                    tab.devtools();
                }
            }
            's' => toggle_split(win),
            'b' => run_command(win, "toggle-bookmarks-bar"),
            'o' => open_page(win, "bookmarks", ""),
            _ => return false,
        }
        return true;
    }

    match l {
        't' => new_tab(win),
        'n' => new_window(win, false, Vec::new()),
        'w' => {
            if let Some(tab) = tab {
                win.close_tab(tab.id, true);
            }
        }
        'l' => win.command(json!({ "type": "focus-omnibox" })),
        'k' => win.command(json!({ "type": "palette" })),
        'r' => {
            if let Some(tab) = tab {
                tab.reload(false);
            }
        }
        'f' => win.command(json!({ "type": "find" })),
        'g' => {
            if let Some(tab) = tab
                && let Some(text) = tab.find_text()
            {
                tab.find_in_page(&text, true, true);
            }
        }
        'b' => run_command(win, "toggle-sidebar"),
        'h' => open_page(win, "history", ""),
        'j' => open_page(win, "downloads", ""),
        'p' => {
            if let Some(tab) = tab {
                tab.print();
            }
        }
        's' => save_page(win),
        'q' => session::quit(&win.app_handle()),
        _ => return false,
    }
    true
}
