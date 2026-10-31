//! What the UI draws, in the shapes of `src/shared/types.ts`. Field names follow the
//! TypeScript types through serde's camelCase renaming.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::settings::Settings;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TabState {
    pub id: u32,
    /// Was the page's webContents id for extension buttons; always null here.
    pub contents_id: Option<u32>,
    pub url: String,
    pub title: String,
    pub favicon: Option<String>,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub audible: bool,
    pub muted: bool,
    pub pinned: bool,
    pub sleeping: bool,
    pub blocked: u32,
    pub zoom: f64,
    pub security: &'static str,
    pub page: Option<&'static str>,
    pub interstitial: Option<Value>,
    pub reader_available: bool,
    pub reader: bool,
    pub find: Option<Value>,
    pub media: Option<&'static str>,
    pub page_lang: Option<String>,
    pub translation: Option<Value>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitState {
    pub left: u32,
    pub right: u32,
    /// Width share of the left pane, 0.2–0.8.
    pub ratio: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceState {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub tab_ids: Vec<u32>,
    pub active_tab_id: Option<u32>,
    pub split: Option<SplitState>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowState {
    pub window_id: u32,
    pub incognito: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub workspaces: Vec<WorkspaceState>,
    pub active_workspace_id: String,
    pub tabs: BTreeMap<u32, TabState>,
    pub permission: Option<Value>,
    /// The active tab's site has the shield switched off.
    pub shield_paused: bool,
    /// Chrome extensions are gone; kept null for the UI.
    pub extensions_partition: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusState {
    pub active: bool,
    pub started_at: u64,
    pub ends_at: u64,
    pub minutes: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    pub status: &'static str,
    pub version: Option<String>,
    pub progress: f64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalState {
    pub focus: FocusState,
    pub update: UpdateState,
    pub downloads: Vec<Value>,
    pub version: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    /// The version people see, with the stage suffix (0.2.0a).
    pub version: String,
    pub stage: &'static str,
    /// The UI's About page still names these after Electron; they now carry Tauri and WebKitGTK.
    pub electron: String,
    pub chrome: String,
    pub node: String,
    pub platform: &'static str,
    pub packaged: bool,
    pub is_default_browser: bool,
    pub hardware_acceleration: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitialData {
    pub state: WindowState,
    pub global: GlobalState,
    pub settings: Settings,
    pub app: AppInfo,
    pub bookmarks: Value,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneRect {
    pub tab_id: u32,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
