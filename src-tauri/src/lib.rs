mod service;
mod storage;
mod transport;
mod watcher;

pub mod core;

use core::{ChannelStatus, TaskSnapshot, TaskState};
use parking_lot::Mutex;
use service::{ContinuationService, ContinuationSnapshot, ContinuationStatusKind};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;
use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    Emitter, Manager, State,
};
use uuid::Uuid;

pub struct AppState {
    service: ContinuationService,
    locale: Mutex<AppLocale>,
}

const TRAY_ICON: Image<'static> = tauri::include_image!("./icons/tray-icon.png");
const TRAY_TASK_LIMIT: usize = 3;
const TRAY_TASK_NAME_MAX_CHARS: usize = 30;
const TRAY_TASK_PREFIX: &str = "open-task:";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

    fn auto_retry(self) -> &'static str {
        match self {
            Self::ZhCn => "自动继续",
            Self::En => "Automatic continuation",
        }
    }

    fn show_window(self) -> &'static str {
        match self {
            Self::ZhCn => "打开 TurnMender",
            Self::En => "Open TurnMender",
        }
    }

    fn settings(self) -> &'static str {
        match self {
            Self::ZhCn => "设置",
            Self::En => "Settings",
        }
    }

    fn view_log(self) -> &'static str {
        match self {
            Self::ZhCn => "查看运行日志",
            Self::En => "View Runtime Log",
        }
    }

    fn attention_heading(self, count: usize) -> String {
        match self {
            Self::ZhCn => format!("待处理任务（{count}）"),
            Self::En => format!("Tasks needing attention ({count})"),
        }
    }

    fn view_all_tasks(self) -> &'static str {
        match self {
            Self::ZhCn => "查看全部待处理任务",
            Self::En => "View All Tasks Needing Attention",
        }
    }

    fn unnamed_task(self) -> &'static str {
        match self {
            Self::ZhCn => "未命名任务",
            Self::En => "Unnamed task",
        }
    }

    fn quit(self) -> &'static str {
        match self {
            Self::ZhCn => "退出 TurnMender",
            Self::En => "Quit TurnMender",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TrayTaskMenuItem {
    task_id: String,
    label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TrayMenuModel {
    locale: AppLocale,
    tone: TrayTone,
    status_label: String,
    auto_retry_enabled: bool,
    attention_count: usize,
    attention_tasks: Vec<TrayTaskMenuItem>,
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

fn is_attention_task(task: &TaskSnapshot) -> bool {
    matches!(
        task.state,
        TaskState::WaitingContinuation | TaskState::Unknown | TaskState::Unavailable
    )
}

fn task_count_label(locale: AppLocale, count: usize, zh_suffix: &str, en_suffix: &str) -> String {
    match locale {
        AppLocale::ZhCn if zh_suffix.is_empty() => format!("{count} 个任务"),
        AppLocale::ZhCn => format!("{count} 个任务{zh_suffix}"),
        AppLocale::En => {
            let noun = if count == 1 { "task" } else { "tasks" };
            if en_suffix.is_empty() {
                format!("{count} {noun}")
            } else {
                format!("{count} {noun} {en_suffix}")
            }
        }
    }
}

fn tray_status_label(snapshot: &ContinuationSnapshot, locale: AppLocale) -> String {
    let critical_count = snapshot
        .tasks
        .iter()
        .filter(|task| matches!(task.state, TaskState::Unknown | TaskState::Unavailable))
        .count();
    let waiting_count = snapshot
        .tasks
        .iter()
        .filter(|task| task.state == TaskState::WaitingContinuation)
        .count();
    let running_count = snapshot
        .tasks
        .iter()
        .filter(|task| task.state == TaskState::Running)
        .count();

    if !snapshot.running {
        return match locale {
            AppLocale::ZhCn => "状态：续行服务已停止".into(),
            AppLocale::En => "Status: continuation stopped".into(),
        };
    }
    if snapshot.status.kind == ContinuationStatusKind::WatchFailed {
        return match locale {
            AppLocale::ZhCn => "状态：任务监听失败".into(),
            AppLocale::En => "Status: monitoring failed".into(),
        };
    }
    if critical_count > 0 {
        return match locale {
            AppLocale::ZhCn => format!("需要处理 · {critical_count} 个任务"),
            AppLocale::En => {
                format!(
                    "Action required · {}",
                    task_count_label(locale, critical_count, "", "")
                )
            }
        };
    }
    if !snapshot.auto_retry_enabled {
        return match (locale, waiting_count) {
            (AppLocale::ZhCn, 0) => "仅监听 · 自动继续已暂停".into(),
            (AppLocale::ZhCn, count) => format!("仅监听 · {count} 个任务等待处理"),
            (AppLocale::En, 0) => "Monitor only · automatic continuation paused".into(),
            (AppLocale::En, count) => format!(
                "Monitor only · {}",
                task_count_label(locale, count, "等待处理", "waiting")
            ),
        };
    }
    if snapshot.channel_status != ChannelStatus::Ready {
        return match (locale, snapshot.channel_status) {
            (AppLocale::ZhCn, ChannelStatus::Unsupported) => "正在监听 · 当前平台仅支持监听".into(),
            (AppLocale::ZhCn, _) => "正在监听 · 消息通道不可用".into(),
            (AppLocale::En, ChannelStatus::Unsupported) => {
                "Monitoring · automatic continuation unsupported".into()
            }
            (AppLocale::En, _) => "Monitoring · messaging unavailable".into(),
        };
    }
    if snapshot.status.kind == ContinuationStatusKind::Preparing {
        return match locale {
            AppLocale::ZhCn => "正在启动任务监听…".into(),
            AppLocale::En => "Starting task monitoring…".into(),
        };
    }
    if snapshot.status.kind == ContinuationStatusKind::Continuing {
        return match locale {
            AppLocale::ZhCn => "正在自动续行…".into(),
            AppLocale::En => "Automatic continuation in progress…".into(),
        };
    }
    if waiting_count > 0 {
        return match locale {
            AppLocale::ZhCn => format!("等待自动续行 · {waiting_count} 个任务"),
            AppLocale::En => format!(
                "Waiting to continue · {}",
                task_count_label(locale, waiting_count, "", "")
            ),
        };
    }

    match (locale, running_count) {
        (AppLocale::ZhCn, 0) => "状态：续行就绪".into(),
        (AppLocale::ZhCn, count) => format!("续行就绪 · {count} 个任务运行中"),
        (AppLocale::En, 0) => "Status: continuation ready".into(),
        (AppLocale::En, count) => format!(
            "Continuation ready · {}",
            task_count_label(locale, count, "运行中", "running")
        ),
    }
}

fn tray_task_reason(
    task: &TaskSnapshot,
    snapshot: &ContinuationSnapshot,
    locale: AppLocale,
) -> String {
    match (locale, task.state) {
        (AppLocale::ZhCn, TaskState::WaitingContinuation) if !snapshot.auto_retry_enabled => {
            "自动继续已暂停".into()
        }
        (AppLocale::En, TaskState::WaitingContinuation) if !snapshot.auto_retry_enabled => {
            "automatic continuation paused".into()
        }
        (AppLocale::ZhCn, TaskState::WaitingContinuation)
            if snapshot.channel_status != ChannelStatus::Ready =>
        {
            "消息通道不可用".into()
        }
        (AppLocale::En, TaskState::WaitingContinuation)
            if snapshot.channel_status != ChannelStatus::Ready =>
        {
            "messaging unavailable".into()
        }
        (AppLocale::ZhCn, TaskState::WaitingContinuation) => "等待自动续行".into(),
        (AppLocale::En, TaskState::WaitingContinuation) => "waiting to continue".into(),
        (AppLocale::ZhCn, TaskState::Unknown)
            if task.continuation_count >= snapshot.automatic_chain_limit =>
        {
            format!("已达 {} 次上限", snapshot.automatic_chain_limit)
        }
        (AppLocale::En, TaskState::Unknown)
            if task.continuation_count >= snapshot.automatic_chain_limit =>
        {
            format!(
                "{}-continuation limit reached",
                snapshot.automatic_chain_limit
            )
        }
        (AppLocale::ZhCn, TaskState::Unknown)
            if snapshot.status.kind == ContinuationStatusKind::ConfirmSend =>
        {
            "发送结果待确认".into()
        }
        (AppLocale::En, TaskState::Unknown)
            if snapshot.status.kind == ContinuationStatusKind::ConfirmSend =>
        {
            "send result needs review".into()
        }
        (AppLocale::ZhCn, TaskState::Unknown) => "需要确认".into(),
        (AppLocale::En, TaskState::Unknown) => "needs review".into(),
        (AppLocale::ZhCn, TaskState::Unavailable) => "需要手动继续".into(),
        (AppLocale::En, TaskState::Unavailable) => "manual continuation required".into(),
        _ => String::new(),
    }
}

fn compact_menu_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_menu_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

fn escape_menu_mnemonics(value: &str) -> String {
    value.replace('&', "&&")
}

fn tray_menu_model(snapshot: &ContinuationSnapshot, locale: AppLocale) -> TrayMenuModel {
    let attention: Vec<_> = snapshot
        .tasks
        .iter()
        .filter(|task| is_attention_task(task))
        .collect();
    let attention_tasks = attention
        .iter()
        .take(TRAY_TASK_LIMIT)
        .map(|task| {
            let name = task
                .task_name
                .as_deref()
                .map(compact_menu_text)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| locale.unnamed_task().to_string());
            let name = truncate_menu_text(&name, TRAY_TASK_NAME_MAX_CHARS);
            let reason = tray_task_reason(task, snapshot, locale);
            TrayTaskMenuItem {
                task_id: task.task_id.clone(),
                label: escape_menu_mnemonics(&format!("{name} · {reason}")),
            }
        })
        .collect();

    TrayMenuModel {
        locale,
        tone: tray_tone(snapshot),
        status_label: tray_status_label(snapshot, locale),
        auto_retry_enabled: snapshot.auto_retry_enabled,
        attention_count: attention.len(),
        attention_tasks,
    }
}

fn set_tray_menu(app: &tauri::AppHandle, model: &TrayMenuModel) -> tauri::Result<()> {
    let menu = Menu::new(app)?;
    let status = MenuItem::with_id(app, "status", &model.status_label, false, None::<&str>)?;
    let auto_retry = CheckMenuItem::with_id(
        app,
        "auto-retry",
        model.locale.auto_retry(),
        true,
        model.auto_retry_enabled,
        None::<&str>,
    )?;
    menu.append(&status)?;
    menu.append(&auto_retry)?;

    if !model.attention_tasks.is_empty() {
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        let heading = MenuItem::with_id(
            app,
            "attention-heading",
            model.locale.attention_heading(model.attention_count),
            false,
            None::<&str>,
        )?;
        menu.append(&heading)?;
        for task in &model.attention_tasks {
            let item = MenuItem::with_id(
                app,
                format!("{TRAY_TASK_PREFIX}{}", task.task_id),
                &task.label,
                true,
                None::<&str>,
            )?;
            menu.append(&item)?;
        }
        if model.attention_count > TRAY_TASK_LIMIT {
            let view_all = MenuItem::with_id(
                app,
                "view-all-tasks",
                model.locale.view_all_tasks(),
                true,
                None::<&str>,
            )?;
            menu.append(&view_all)?;
        }
    }

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    let show = MenuItem::with_id(app, "show", model.locale.show_window(), true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", model.locale.settings(), true, None::<&str>)?;
    let view_log = MenuItem::with_id(app, "view-log", model.locale.view_log(), true, None::<&str>)?;
    menu.append(&show)?;
    menu.append(&settings)?;
    menu.append(&view_log)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    let quit = MenuItem::with_id(app, "quit", model.locale.quit(), true, None::<&str>)?;
    menu.append(&quit)?;

    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(menu))?;
    }
    Ok(())
}

fn apply_tray_model(app: &tauri::AppHandle, model: &TrayMenuModel) -> tauri::Result<()> {
    set_tray_menu(app, model)?;
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_icon_with_as_template(Some(colored_tray_icon(model.tone.color())), false)?;
        tray.set_tooltip(Some(format!("TurnMender · {}", model.status_label)))?;
    }
    Ok(())
}

fn start_tray_status_sync(app: &tauri::AppHandle) {
    let app_handle = app.clone();
    thread::spawn(move || {
        let mut current_model: Option<TrayMenuModel> = None;
        loop {
            let state = app_handle.state::<AppState>();
            let snapshot = state.service.snapshot();
            let locale = *state.locale.lock();
            let model = tray_menu_model(&snapshot, locale);
            if current_model.as_ref() != Some(&model)
                && apply_tray_model(&app_handle, &model).is_ok()
            {
                current_model = Some(model);
            }
            thread::sleep(Duration::from_secs(1));
        }
    });
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn show_settings_window(app: &tauri::AppHandle) {
    show_main_window(app);
    let _ = app.emit_to("main", "open-settings", ());
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
fn set_automatic_chain_limit(limit: u32, state: State<'_, AppState>) -> u32 {
    state.service.set_automatic_chain_limit(limit)
}

#[tauri::command]
fn set_continuation_settings(
    automatic_chain_limit: u32,
    retry_message: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .service
        .set_continuation_settings(automatic_chain_limit, retry_message)
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
    let snapshot = state.service.snapshot();
    let model = tray_menu_model(&snapshot, locale);
    apply_tray_model(&app, &model).map_err(|error| error.to_string())
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

fn open_service_log(service: &ContinuationService) -> Result<(), String> {
    let log_path = PathBuf::from(service.snapshot().log_path);
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

#[tauri::command]
fn open_log(state: State<'_, AppState>) -> Result<(), String> {
    open_service_log(&state.service)
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
    open_codex_thread_by_id(&task_id)
}

fn open_codex_thread_by_id(task_id: &str) -> Result<(), String> {
    let url = codex_thread_url(task_id)?;
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
            set_automatic_chain_limit,
            set_continuation_settings,
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
            app.state::<AppState>().service.start();
            let locale = *app.state::<AppState>().locale.lock();
            let snapshot = app.state::<AppState>().service.snapshot();
            let model = tray_menu_model(&snapshot, locale);
            apply_tray_model(app.handle(), &model)?;
            if let Some(tray) = app.tray_by_id("main") {
                tray.on_menu_event(|app, event| {
                    let item_id = event.id.as_ref();
                    match item_id {
                        "show" | "view-all-tasks" => show_main_window(app),
                        "settings" => show_settings_window(app),
                        "auto-retry" => {
                            let state = app.state::<AppState>();
                            let enabled = !state.service.snapshot().auto_retry_enabled;
                            state.service.set_auto_retry(enabled);
                        }
                        "view-log" => {
                            let state = app.state::<AppState>();
                            let _ = open_service_log(&state.service);
                        }
                        "quit" => app.exit(0),
                        _ => {
                            if let Some(task_id) = item_id.strip_prefix(TRAY_TASK_PREFIX) {
                                if open_codex_thread_by_id(task_id).is_err() {
                                    show_main_window(app);
                                }
                            }
                        }
                    }
                });
            }
            start_tray_status_sync(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running TurnMender");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::ContinuationStatus;

    fn task(task_id: &str, state: TaskState, name: &str, continuation_count: u32) -> TaskSnapshot {
        TaskSnapshot {
            task_id: task_id.into(),
            task_name: Some(name.into()),
            project_path: None,
            state,
            latest_turn_id: None,
            last_activity_at: Some(1.0),
            continuation_count,
            pending_failure: None,
            channel_status: ChannelStatus::Ready,
        }
    }

    fn snapshot(tasks: Vec<TaskSnapshot>) -> ContinuationSnapshot {
        ContinuationSnapshot {
            running: true,
            auto_retry_enabled: true,
            automatic_chain_limit: 10,
            automatic_chain_limit_min: 1,
            automatic_chain_limit_max: 100,
            retry_message: "继续".into(),
            default_retry_message: "继续".into(),
            retry_message_max_chars: 1000,
            platform: "macos".into(),
            session_root: "/tmp/sessions".into(),
            log_path: "/tmp/turnmender.log".into(),
            channel_status: ChannelStatus::Ready,
            status: ContinuationStatus {
                kind: ContinuationStatusKind::Watching,
                detail: None,
            },
            tasks,
        }
    }

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

    #[test]
    fn tray_menu_lists_only_three_attention_tasks() {
        let snapshot = snapshot(vec![
            task("task-1", TaskState::Unknown, "修复 A&B", 0),
            task("task-2", TaskState::Unavailable, "任务二", 0),
            task("task-3", TaskState::WaitingContinuation, "任务三", 0),
            task("task-4", TaskState::Unknown, "任务四", 10),
            task("task-idle", TaskState::Idle, "空闲任务", 0),
        ]);

        let model = tray_menu_model(&snapshot, AppLocale::ZhCn);

        assert_eq!(model.attention_count, 4);
        assert_eq!(model.attention_tasks.len(), TRAY_TASK_LIMIT);
        assert_eq!(model.attention_tasks[0].task_id, "task-1");
        assert!(model.attention_tasks[0].label.contains("A&&B"));
        assert!(!model
            .attention_tasks
            .iter()
            .any(|task| task.task_id == "task-idle"));
    }

    #[test]
    fn tray_status_explains_why_waiting_tasks_need_attention() {
        let mut snapshot = snapshot(vec![task(
            "task-1",
            TaskState::WaitingContinuation,
            "等待中的任务",
            0,
        )]);
        snapshot.auto_retry_enabled = false;

        let model = tray_menu_model(&snapshot, AppLocale::ZhCn);

        assert_eq!(model.tone, TrayTone::Yellow);
        assert_eq!(model.status_label, "仅监听 · 1 个任务等待处理");
        assert!(model.attention_tasks[0].label.ends_with("自动继续已暂停"));
    }

    #[test]
    fn tray_status_uses_english_singular_task_label() {
        let snapshot = snapshot(vec![task("task-1", TaskState::Running, "Running task", 0)]);

        let model = tray_menu_model(&snapshot, AppLocale::En);

        assert_eq!(model.status_label, "Continuation ready · 1 task running");
    }
}
