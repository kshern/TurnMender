mod service;
mod storage;
mod transport;
mod watcher;

pub mod core;

use core::{ChannelStatus, TaskState};
use parking_lot::Mutex;
use service::{ContinuationService, ContinuationSnapshot, ContinuationStatusKind};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    Manager, State,
};
use uuid::Uuid;

pub struct AppState {
    service: ContinuationService,
    locale: Mutex<AppLocale>,
}

const TRAY_ICON: Image<'static> = tauri::include_image!("./icons/tray-icon.png");

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppLocale {
    ZhCn,
    En,
}

impl AppLocale {
    fn detect() -> Self {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .filter_map(|key| std::env::var(key).ok())
            .find(|value| !value.trim().is_empty())
            .map(|value| Self::from_code(&value))
            .unwrap_or(Self::En)
    }

    fn from_code(value: &str) -> Self {
        if value.trim().to_ascii_lowercase().starts_with("zh") {
            Self::ZhCn
        } else {
            Self::En
        }
    }

    fn show_window(self) -> &'static str {
        match self {
            Self::ZhCn => "显示窗口",
            Self::En => "Show Window",
        }
    }

    fn quit(self) -> &'static str {
        match self {
            Self::ZhCn => "退出",
            Self::En => "Quit",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TrayTone {
    Green,
    Yellow,
    Red,
}

impl TrayTone {
    fn color(self) -> [u8; 3] {
        match self {
            Self::Green => [39, 183, 126],
            Self::Yellow => [230, 161, 46],
            Self::Red => [223, 91, 98],
        }
    }

    fn label(self, locale: AppLocale) -> &'static str {
        match (self, locale) {
            (Self::Green, AppLocale::ZhCn) => "续行就绪",
            (Self::Yellow, AppLocale::ZhCn) => "需要留意",
            (Self::Red, AppLocale::ZhCn) => "需要处理",
            (Self::Green, AppLocale::En) => "Continuation ready",
            (Self::Yellow, AppLocale::En) => "Needs attention",
            (Self::Red, AppLocale::En) => "Action required",
        }
    }
}

fn tray_tone(snapshot: &ContinuationSnapshot) -> TrayTone {
    if !snapshot.running {
        return TrayTone::Red;
    }
    if snapshot.status.kind == ContinuationStatusKind::WatchFailed
        || snapshot
            .tasks
            .iter()
            .any(|task| matches!(task.state, TaskState::Unknown | TaskState::Unavailable))
    {
        return TrayTone::Red;
    }
    if snapshot.channel_status != ChannelStatus::Ready
        || !snapshot.auto_retry_enabled
        || snapshot
            .tasks
            .iter()
            .any(|task| task.state == TaskState::WaitingContinuation)
    {
        return TrayTone::Yellow;
    }
    TrayTone::Green
}

fn colored_tray_icon(color: [u8; 3]) -> Image<'static> {
    let rgba = TRAY_ICON
        .rgba()
        .chunks_exact(4)
        .flat_map(|pixel| [color[0], color[1], color[2], pixel[3]])
        .collect();
    Image::new_owned(rgba, TRAY_ICON.width(), TRAY_ICON.height())
}

fn set_tray_menu(app: &tauri::AppHandle, locale: AppLocale) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", locale.show_window(), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", locale.quit(), true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(menu))?;
    }
    Ok(())
}

fn start_tray_status_sync(app: &tauri::AppHandle) {
    let app_handle = app.clone();
    thread::spawn(move || {
        let mut current_status = None;
        loop {
            let state = app_handle.state::<AppState>();
            let snapshot = state.service.snapshot();
            let tone = tray_tone(&snapshot);
            let locale = *state.locale.lock();
            if current_status != Some((tone, locale)) {
                if let Some(tray) = app_handle.tray_by_id("main") {
                    let _ = tray
                        .set_icon_with_as_template(Some(colored_tray_icon(tone.color())), false);
                    let _ = tray.set_tooltip(Some(format!("TurnMender · {}", tone.label(locale))));
                    current_status = Some((tone, locale));
                }
            }
            thread::sleep(Duration::from_secs(1));
        }
    });
}

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> ContinuationSnapshot {
    state.service.snapshot()
}

#[tauri::command]
fn set_auto_retry(enabled: bool, state: State<'_, AppState>) {
    state.service.set_auto_retry(enabled);
}

#[tauri::command]
fn dismiss_task(task_id: String, state: State<'_, AppState>) -> bool {
    state.service.dismiss_task(&task_id)
}

#[tauri::command]
fn set_locale(
    locale: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let locale = AppLocale::from_code(&locale);
    *state.locale.lock() = locale;
    set_tray_menu(&app, locale).map_err(|error| error.to_string())?;
    let snapshot = state.service.snapshot();
    if let Some(tray) = app.tray_by_id("main") {
        let tone = tray_tone(&snapshot);
        tray.set_tooltip(Some(format!("TurnMender · {}", tone.label(locale))))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn open_in_default_app(path: &Path) -> Result<(), String> {
    Command::new("open")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开日志：{error}"))
}

#[cfg(target_os = "windows")]
fn open_in_default_app(path: &Path) -> Result<(), String> {
    Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开日志：{error}"))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn open_in_default_app(path: &Path) -> Result<(), String> {
    Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开日志：{error}"))
}

#[tauri::command]
fn open_log(state: State<'_, AppState>) -> Result<(), String> {
    let log_path = PathBuf::from(state.service.snapshot().log_path);
    if let Some(parent) = Path::new(&log_path).parent() {
        fs::create_dir_all(parent).map_err(|error| format!("无法创建日志目录：{error}"))?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| format!("无法读取日志：{error}"))?;
    open_in_default_app(Path::new(&log_path))
}

fn codex_thread_url(task_id: &str) -> Result<String, String> {
    let task_id = Uuid::parse_str(task_id.trim()).map_err(|_| "任务 ID 无效".to_string())?;
    Ok(format!("codex://threads/{}", task_id.hyphenated()))
}

#[cfg(target_os = "macos")]
fn open_external_url(url: &str) -> Result<(), String> {
    Command::new("open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开 Codex 任务：{error}"))
}

#[cfg(target_os = "windows")]
fn open_external_url(url: &str) -> Result<(), String> {
    Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开 Codex 任务：{error}"))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn open_external_url(url: &str) -> Result<(), String> {
    Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开 Codex 任务：{error}"))
}

#[tauri::command]
fn open_codex_thread(task_id: String) -> Result<(), String> {
    let url = codex_thread_url(&task_id)?;
    open_external_url(&url)
}

pub fn run() {
    let initial_locale = AppLocale::detect();
    tauri::Builder::default()
        .manage(AppState {
            service: ContinuationService::new(),
            locale: Mutex::new(initial_locale),
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            set_auto_retry,
            dismiss_task,
            set_locale,
            open_codex_thread,
            open_log
        ])
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            let locale = *app.state::<AppState>().locale.lock();
            set_tray_menu(app.handle(), locale)?;
            if let Some(tray) = app.tray_by_id("main") {
                tray.on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.unminimize();
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                });
            }
            app.state::<AppState>().service.start();
            start_tray_status_sync(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running TurnMender");
}

#[cfg(test)]
mod tests {
    use super::codex_thread_url;

    #[test]
    fn builds_codex_thread_deep_link_from_task_id() {
        assert_eq!(
            codex_thread_url("123e4567-e89b-12d3-a456-426614174000").unwrap(),
            "codex://threads/123e4567-e89b-12d3-a456-426614174000"
        );
    }

    #[test]
    fn rejects_invalid_codex_thread_id() {
        assert!(codex_thread_url("not-a-task-id").is_err());
    }
}
