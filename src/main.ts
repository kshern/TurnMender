import { invoke } from "@tauri-apps/api/core";
import brandLogoUrl from "../src-tauri/icons/128x128.png";
import { getLocale, setLocale, t } from "./i18n";
import "./styles.css";

type TaskState =
  | "running"
  | "waiting_continuation"
  | "completed_with_output"
  | "idle"
  | "unknown"
  | "unavailable";

type ChannelStatus = "ready" | "unavailable" | "unsupported" | "unknown";
type HealthTone = "green" | "yellow" | "red";
type ContinuationStatusKind =
  | "preparing"
  | "watch_failed"
  | "watching"
  | "watching_unsupported"
  | "watching_channel_unavailable"
  | "task_waiting"
  | "chain_protected"
  | "manual_continue"
  | "continuing"
  | "continued"
  | "confirm_send"
  | "stopped";

interface ContinuationStatus {
  kind: ContinuationStatusKind;
  task_name: string | null;
  detail: string | null;
}

interface TaskSnapshot {
  task_id: string;
  task_name: string | null;
  project_path: string | null;
  state: TaskState;
  latest_turn_id: string | null;
  last_activity_at: number | null;
  continuation_count: number;
  pending_failure: string | null;
  channel_status: ChannelStatus;
}

interface ContinuationSnapshot {
  running: boolean;
  auto_retry_enabled: boolean;
  platform: string;
  session_root: string;
  log_path: string;
  channel_status: ChannelStatus;
  status: ContinuationStatus;
  tasks: TaskSnapshot[];
}

const state = {
  snapshot: null as ContinuationSnapshot | null,
  snapshotSignature: "",
  busy: false,
};

const appRoot = document.querySelector<HTMLDivElement>("#app");
if (!appRoot) throw new Error("app root missing");
const app = appRoot;

interface ViewState {
  pageScrollTop: number;
  taskScrollTop: number;
}

function captureViewState(): ViewState {
  return {
    pageScrollTop: app.scrollTop,
    taskScrollTop: document.querySelector<HTMLElement>(".task-list")?.scrollTop ?? 0,
  };
}

function restoreViewState(viewState: ViewState): void {
  const taskList = document.querySelector<HTMLElement>(".task-list");
  if (taskList) taskList.scrollTop = viewState.taskScrollTop;
  app.scrollTop = viewState.pageScrollTop;
}

const icons: Record<string, string> = {
  activity:
    '<path d="M3 12h3l2-6 4 12 2-6h4"/><path d="M3 19h18" opacity=".22"/>',
  file: '<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Z"/><path d="M14 2v6h6M8 13h8M8 17h6"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z"/>',
  globe:
    '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.3 2.5 3.5 5.5 3.5 9s-1.2 6.5-3.5 9c-2.3-2.5-3.5-5.5-3.5-9S9.7 5.5 12 3Z"/>',
  shield:
    '<path d="M12 3 19 6v5c0 4.7-2.7 8-7 10-4.3-2-7-5.3-7-10V6l7-3Z"/><path d="m9 12 2 2 4-4"/>',
  open: '<path d="M15 3h6v6M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>',
  x: '<path d="m7 7 10 10M17 7 7 17"/>',
};

function icon(name: string, className = "icon"): string {
  return `<svg class="${className}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${icons[name] ?? ""}</svg>`;
}

function escapeHTML(value: string): string {
  return value.replace(/[&<>"']/g, (character) => {
    const entities: Record<string, string> = {
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      '"': "&quot;",
      "'": "&#39;",
    };
    return entities[character] ?? character;
  });
}

function formatTime(timestamp: number | null): string {
  if (!timestamp) return t("time.noActivity");
  return new Date(timestamp * 1000).toLocaleTimeString(getLocale(), {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function projectName(projectPath: string | null): string {
  if (!projectPath?.trim()) return t("task.projectUnknown");
  const trimmed = projectPath.trim().replace(/[\\/]+$/, "");
  if (!trimmed) return projectPath.trim();
  return trimmed.split(/[\\/]/).pop() || trimmed;
}

function stateLabel(value: TaskState): string {
  const labels: Record<TaskState, string> = {
    running: t("task.state.running"),
    waiting_continuation: t("task.state.waitingContinuation"),
    completed_with_output: t("task.state.completedWithOutput"),
    idle: t("task.state.idle"),
    unknown: t("task.state.unknown"),
    unavailable: t("task.state.unavailable"),
  };
  return labels[value];
}

function channelLabel(value: ChannelStatus): string {
  const labels: Record<ChannelStatus, string> = {
    ready: t("channel.ready"),
    unavailable: t("channel.unavailable"),
    unsupported: t("channel.unsupported"),
    unknown: t("channel.unknown"),
  };
  return labels[value];
}

function channelTone(value: ChannelStatus): HealthTone {
  if (value === "ready") return "green";
  if (value === "unknown") return "yellow";
  return "red";
}

function healthTone(snapshot: ContinuationSnapshot): HealthTone {
  if (!snapshot.running) return "red";
  if (
    snapshot.status.kind === "watch_failed" ||
    snapshot.tasks.some((task) => task.state === "unknown" || task.state === "unavailable")
  ) {
    return "red";
  }
  if (
    snapshot.channel_status !== "ready" ||
    !snapshot.auto_retry_enabled ||
    snapshot.tasks.some((task) => task.state === "waiting_continuation")
  ) {
    return "yellow";
  }
  return "green";
}

function taskDescription(task: TaskSnapshot): string {
  if (task.state === "waiting_continuation") return t("task.description.waitingContinuation");
  if (task.state === "unknown") return t("task.description.unknown");
  if (task.state === "unavailable") return t("task.description.unavailable");
  return t("task.description.lastActivity");
}

function statusText(status: ContinuationStatus): string {
  const task = status.task_name?.trim() || t("task.unnamed");
  const values = { task, detail: status.detail || t("status.unknown") };
  const keys: Record<ContinuationStatusKind, Parameters<typeof t>[0]> = {
    preparing: "status.preparing",
    watch_failed: "status.watchFailed",
    watching: "status.watching",
    watching_unsupported: "status.watchingUnsupported",
    watching_channel_unavailable: "status.watchingChannelUnavailable",
    task_waiting: "status.taskWaiting",
    chain_protected: "status.chainProtected",
    manual_continue: "status.manualContinue",
    continuing: "status.continuing",
    continued: "status.continued",
    confirm_send: "status.confirmSend",
    stopped: "status.stopped",
  };
  return t(keys[status.kind] ?? "status.unknown", values);
}

function render(): void {
  const snapshot = state.snapshot;
  if (!snapshot) {
    app.innerHTML = `<main class="shell"><div class="loading"><span class="loading-spinner"></span>${t("loading.snapshot")}</div></main>`;
    return;
  }

  const tone = healthTone(snapshot);
  const viewState = captureViewState();
  const runningTasks = snapshot.tasks.filter((task) => task.state === "running").length;
  const attentionTasks = snapshot.tasks.filter((task) =>
    ["waiting_continuation", "unknown", "unavailable"].includes(task.state),
  ).length;
  const taskRows = snapshot.tasks.length
    ? snapshot.tasks
        .map((task) => {
          const attention = ["waiting_continuation", "unknown", "unavailable"].includes(task.state);
          const taskName = task.task_name?.trim() || t("task.unnamed");
          const projectPath = task.project_path?.trim() || null;
          const project = projectName(projectPath);
          const projectTitle = projectPath
            ? t("task.projectPathTitle", { path: projectPath })
            : t("task.projectUnknown");
          const channelWarning = task.channel_status === "ready" || task.state === "unavailable"
            ? ""
            : `<span class="task-channel tone-${channelTone(task.channel_status)}">${escapeHTML(channelLabel(task.channel_status))}</span>`;
          const taskTone =
            task.state === "unknown" || task.state === "unavailable"
              ? "red"
              : task.state === "waiting_continuation"
                ? "yellow"
                : "green";
          return `
            <article class="task-row ${attention ? "attention" : ""}">
              <span class="task-state-mark tone-${taskTone}"></span>
              <div class="task-info">
                <div class="task-title-line">
                  <span class="task-name" title="${escapeHTML(t("task.idTitle", { id: task.task_id }))}">${escapeHTML(taskName)}</span>
                  <span class="task-state state-${task.state}">${escapeHTML(stateLabel(task.state))}</span>
                </div>
                <div class="task-subline">
                  <span class="task-project" title="${escapeHTML(projectTitle)}">${icon("folder")}<span>${escapeHTML(project)}</span></span>
                  <span class="task-subline-separator">·</span>
                  <span>${escapeHTML(taskDescription(task))}</span>
                  <span class="task-subline-separator">·</span>
                  <span>${escapeHTML(formatTime(task.last_activity_at))}</span>
                  <span class="task-subline-separator">·</span>
                  <span class="task-continuation-count ${task.continuation_count > 0 ? "active" : ""}" title="${escapeHTML(t("task.continuationCountTitle"))}">${escapeHTML(t("task.continuationCount", { count: task.continuation_count }))}</span>
                </div>
              </div>
              ${channelWarning}
              <button class="task-open" type="button" data-task-id="${escapeHTML(task.task_id)}" aria-label="${escapeHTML(t("task.openAria", { task: taskName }))}" title="${escapeHTML(t("task.openAria", { task: taskName }))}">
                ${icon("open")}<span>${escapeHTML(t("task.open"))}</span>
              </button>
              <button class="task-dismiss" type="button" data-task-id="${escapeHTML(task.task_id)}" aria-label="${escapeHTML(t("task.dismissAria", { task: taskName }))}" title="${escapeHTML(t("task.dismiss"))}">
                ${icon("x")}
              </button>
            </article>`;
        })
        .join("")
    : `<div class="empty-state">${icon("shield", "empty-icon")}<strong>${t("empty.title")}</strong><span>${t("empty.description")}</span></div>`;

  const currentLocale = getLocale();
  const languageAria = escapeHTML(t("language.change"));
  const logLabel = escapeHTML(t("actions.viewLog"));

  app.innerHTML = `
    <main class="shell">
      <header class="topbar">
        <div class="brand">
          <img class="brand-mark" src="${brandLogoUrl}" alt="" draggable="false" />
          <div>
            <h1>TurnMender</h1>
            <p>${t("brand.subtitle")}</p>
          </div>
        </div>
        <div class="top-actions">
          <div class="language-picker">
            <button id="language-toggle" class="language-button" type="button" aria-label="${languageAria}" aria-haspopup="menu" aria-expanded="false" aria-controls="language-menu">${icon("globe")}</button>
            <div id="language-menu" class="language-menu" role="menu" aria-label="${languageAria}" hidden>
              <button class="language-option" type="button" role="menuitemradio" aria-checked="${currentLocale === "zh-CN"}" data-locale="zh-CN" lang="zh-CN"><span>${t("language.chinese")}</span><span class="language-selected-mark" aria-hidden="true"></span></button>
              <button class="language-option" type="button" role="menuitemradio" aria-checked="${currentLocale === "en"}" data-locale="en" lang="en"><span>${t("language.english")}</span><span class="language-selected-mark" aria-hidden="true"></span></button>
            </div>
          </div>
          <button id="view-log" class="log-button" type="button" aria-label="${logLabel}" data-tooltip="${logLabel}">${icon("file")}</button>
          <div class="top-retry-control ${snapshot.auto_retry_enabled ? "on" : "off"}">
            <span class="retry-state">${snapshot.auto_retry_enabled ? t("retry.on") : t("retry.off")}</span>
            <button id="auto-retry" class="toggle-switch ${snapshot.auto_retry_enabled ? "on" : "off"}" type="button" aria-label="${snapshot.auto_retry_enabled ? t("retry.disableAria") : t("retry.enableAria")}" aria-pressed="${snapshot.auto_retry_enabled}" ${state.busy ? "disabled" : ""}>
              <span></span>
            </button>
          </div>
        </div>
      </header>

      <section class="overview-grid" aria-label="${t("overview.ariaLabel")}">
        <article class="overview-card current-card tone-${tone}">
          <div class="overview-heading">
            <span class="overview-kicker"><span class="pulse-mark"></span>${t("overview.currentStatus")}</span>
          </div>
          <h2>${escapeHTML(statusText(snapshot.status))}</h2>
          <div class="current-card-footer">
            <div><strong>${snapshot.tasks.length}</strong><span>${t("overview.tasks")}</span></div>
            <span class="footer-divider"></span>
            <div><strong>${runningTasks}</strong><span>${t("overview.running")}</span></div>
            <span class="footer-divider"></span>
            <div><strong>${attentionTasks}</strong><span>${t("overview.needsAttention")}</span></div>
            <span class="current-orbit">${icon("activity")}</span>
          </div>
          <span class="current-card-glow"></span>
        </article>
      </section>

      <section class="panel task-panel">
        <div class="panel-heading">
          <div class="panel-title">
            <span class="panel-icon blue">${icon("activity")}</span>
            <div><h2>${t("tasks.title")}</h2><p>${t("tasks.autoUpdates")}</p></div>
          </div>
        </div>
        <div class="task-list">${taskRows}</div>
      </section>

    </main>`;

  restoreViewState(viewState);

  const autoRetry = document.querySelector<HTMLButtonElement>("#auto-retry");
  autoRetry?.addEventListener("click", async () => {
    state.busy = true;
    autoRetry.disabled = true;
    try {
      await invoke("set_auto_retry", { enabled: !snapshot.auto_retry_enabled });
      await loadSnapshot();
    } finally {
      state.busy = false;
      const toggle = document.querySelector<HTMLButtonElement>("#auto-retry");
      if (toggle) toggle.disabled = false;
    }
  });

  const languageToggle = document.querySelector<HTMLButtonElement>("#language-toggle");
  const languageMenu = document.querySelector<HTMLElement>("#language-menu");
  languageToggle?.addEventListener("click", (event) => {
    event.stopPropagation();
    if (!languageMenu) return;
    const willOpen = languageMenu.hidden;
    languageMenu.hidden = !willOpen;
    languageToggle.setAttribute("aria-expanded", String(willOpen));
    if (willOpen) {
      languageMenu.querySelector<HTMLButtonElement>('[aria-checked="true"]')?.focus();
    }
  });

  document.querySelectorAll<HTMLButtonElement>(".language-option").forEach((button) => {
    button.addEventListener("click", () => {
      const locale = button.dataset.locale;
      if (locale !== "zh-CN" && locale !== "en") return;
      if (locale === getLocale()) {
        closeLanguageMenu();
        languageToggle?.focus();
        return;
      }
      setLocale(locale);
      render();
      void invoke("set_locale", { locale }).catch((error) => console.error(error));
    });
  });

  const viewLog = document.querySelector<HTMLButtonElement>("#view-log");
  viewLog?.addEventListener("click", async () => {
    viewLog.disabled = true;
    try {
      await invoke("open_log");
    } catch (error) {
      console.error(error);
      const errorLabel = t("actions.openFailed");
      viewLog.dataset.tooltip = errorLabel;
      viewLog.setAttribute("aria-label", errorLabel);
      window.setTimeout(() => {
        if (!viewLog.isConnected) return;
        const restoredLabel = t("actions.viewLog");
        viewLog.dataset.tooltip = restoredLabel;
        viewLog.setAttribute("aria-label", restoredLabel);
      }, 1600);
    } finally {
      viewLog.disabled = false;
    }
  });

  document.querySelectorAll<HTMLButtonElement>(".task-dismiss").forEach((button) => {
    button.addEventListener("click", async () => {
      const taskId = button.dataset.taskId;
      if (!taskId) return;
      button.disabled = true;
      try {
        await invoke("dismiss_task", { taskId });
        await loadSnapshot();
      } catch (error) {
        console.error(error);
        button.disabled = false;
      }
    });
  });

  document.querySelectorAll<HTMLButtonElement>(".task-open").forEach((button) => {
    button.addEventListener("click", async () => {
      const taskId = button.dataset.taskId;
      const label = button.querySelector<HTMLSpanElement>("span");
      if (!taskId) return;
      button.disabled = true;
      try {
        await invoke("open_codex_thread", { taskId });
      } catch (error) {
        console.error(error);
        button.classList.add("error");
        if (label) label.textContent = t("actions.openFailed");
        window.setTimeout(() => {
          if (!button.isConnected) return;
          button.classList.remove("error");
          if (label) label.textContent = t("task.open");
        }, 1600);
      } finally {
        button.disabled = false;
      }
    });
  });
}

function closeLanguageMenu(): void {
  const languageMenu = document.querySelector<HTMLElement>("#language-menu");
  const languageToggle = document.querySelector<HTMLButtonElement>("#language-toggle");
  if (!languageMenu || languageMenu.hidden) return;
  languageMenu.hidden = true;
  languageToggle?.setAttribute("aria-expanded", "false");
}

document.addEventListener("click", (event) => {
  const picker = document.querySelector<HTMLElement>(".language-picker");
  if (event.target instanceof Node && picker?.contains(event.target)) return;
  closeLanguageMenu();
});

document.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  const languageMenu = document.querySelector<HTMLElement>("#language-menu");
  if (!languageMenu || languageMenu.hidden) return;
  closeLanguageMenu();
  document.querySelector<HTMLButtonElement>("#language-toggle")?.focus();
});

async function loadSnapshot(): Promise<void> {
  try {
    const snapshot = await invoke<ContinuationSnapshot>("get_snapshot");
    const signature = JSON.stringify(snapshot);
    if (signature === state.snapshotSignature) return;
    state.snapshot = snapshot;
    state.snapshotSignature = signature;
    render();
  } catch (error) {
    console.error(error);
  }
}

render();
void invoke("set_locale", { locale: getLocale() }).catch((error) => console.error(error));
void loadSnapshot();
window.setInterval(() => void loadSnapshot(), 3000);
