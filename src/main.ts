import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import brandLogoUrl from "../src-tauri/icons/128x128.png";
import { getLocale, setLocale, t, type Locale } from "./i18n";
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
  automatic_chain_limit: number;
  automatic_chain_limit_min: number;
  automatic_chain_limit_max: number;
  retry_message: string;
  default_retry_message: string;
  retry_message_max_chars: number;
  platform: string;
  session_root: string;
  log_path: string;
  channel_status: ChannelStatus;
  status: ContinuationStatus;
  tasks: TaskSnapshot[];
}

interface SettingsDraft {
  automaticChainLimit: string;
  retryMessage: string;
  locale: Locale;
}

const state = {
  snapshot: null as ContinuationSnapshot | null,
  snapshotSignature: "",
  busy: false,
  settingsOpen: false,
  settingsDraft: null as SettingsDraft | null,
  settingsError: "",
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
  settings:
    '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.83 2.83-.06-.06a1.7 1.7 0 0 0-1.88-.34 1.7 1.7 0 0 0-1.03 1.56V21h-4v-.08A1.7 1.7 0 0 0 8.96 19.4a1.7 1.7 0 0 0-1.88.34l-.06.06-2.83-2.83.06-.06A1.7 1.7 0 0 0 4.6 15 1.7 1.7 0 0 0 3.08 14H3v-4h.08A1.7 1.7 0 0 0 4.6 8.96a1.7 1.7 0 0 0-.34-1.88l-.06-.06L7.03 4.2l.06.06A1.7 1.7 0 0 0 8.96 4.6 1.7 1.7 0 0 0 10 3.08V3h4v.08a1.7 1.7 0 0 0 1.04 1.52 1.7 1.7 0 0 0 1.88-.34l.06-.06 2.83 2.83-.06.06a1.7 1.7 0 0 0-.34 1.88A1.7 1.7 0 0 0 20.92 10H21v4h-.08A1.7 1.7 0 0 0 19.4 15Z"/>',
  chevron: '<path d="m7 10 5 5 5-5"/>',
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

function statusText(status: ContinuationStatus, automaticChainLimit: number): string {
  // The overview card is intentionally global. Task names belong in the
  // recent-task list, so a session title must never become the headline here.
  const values = {
    detail: status.detail || t("status.unknown"),
    limit: automaticChainLimit,
  };
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

function settingsDialog(snapshot: ContinuationSnapshot): string {
  const draft = state.settingsDraft;
  if (!state.settingsOpen || !draft) return "";

  const messageLength = [...draft.retryMessage].length;
  const limitAria = escapeHTML(t("retry.limitAria", {
    min: snapshot.automatic_chain_limit_min,
    max: snapshot.automatic_chain_limit_max,
  }));
  const error = state.settingsError
    ? `<div class="settings-error" role="alert">${escapeHTML(state.settingsError)}</div>`
    : "";

  return `
    <div class="settings-backdrop" data-settings-backdrop>
      <section class="settings-dialog" role="dialog" aria-modal="true" aria-labelledby="settings-title" aria-describedby="settings-description">
        <header class="settings-header">
          <div>
            <h2 id="settings-title">${t("settings.title")}</h2>
            <p id="settings-description">${t("settings.description")}</p>
          </div>
          <button id="settings-close" class="settings-close" type="button" aria-label="${escapeHTML(t("settings.closeAria"))}" ${state.busy ? "disabled" : ""}>${icon("x")}</button>
        </header>

        <form id="settings-form" class="settings-form">
          <div class="settings-section">
            <label class="setting-field" for="settings-chain-limit">
              <span>${t("settings.limitLabel")}</span>
              <span class="setting-inline-control">
                <input id="settings-chain-limit" class="settings-number-input" type="number" min="${snapshot.automatic_chain_limit_min}" max="${snapshot.automatic_chain_limit_max}" step="1" inputmode="numeric" value="${escapeHTML(draft.automaticChainLimit)}" aria-label="${limitAria}" ${state.busy ? "disabled" : ""} />
                <span>${t("retry.limitUnit")}</span>
              </span>
              <small>${t("settings.limitHint", { min: snapshot.automatic_chain_limit_min, max: snapshot.automatic_chain_limit_max })}</small>
            </label>

            <label class="setting-field setting-message-field" for="settings-retry-message">
              <span>${t("settings.messageLabel")}</span>
              <textarea id="settings-retry-message" rows="6" spellcheck="true" ${state.busy ? "disabled" : ""}>${escapeHTML(draft.retryMessage)}</textarea>
              <span class="message-field-footer">
                <small>${t("settings.messageHint")}</small>
                <span id="settings-message-count" class="message-count ${messageLength > snapshot.retry_message_max_chars ? "over-limit" : ""}">${t("settings.messageCount", { count: messageLength, max: snapshot.retry_message_max_chars })}</span>
              </span>
            </label>
            <button id="settings-reset-message" class="reset-button" type="button" ${state.busy ? "disabled" : ""}>${t("settings.resetDefault")}</button>
          </div>

          <div class="settings-section settings-language-section">
            <div class="setting-row settings-language-row">
              <label id="settings-language-label">${t("settings.languageLabel")}</label>
              <div class="settings-language-picker">
                <button id="settings-language-toggle" class="settings-select-button" type="button" aria-labelledby="settings-language-label settings-language-value" aria-haspopup="listbox" aria-expanded="false" aria-controls="settings-language-menu" ${state.busy ? "disabled" : ""}>
                  <span id="settings-language-value">${draft.locale === "zh-CN" ? t("language.chinese") : t("language.english")}</span>
                  ${icon("chevron")}
                </button>
                <div id="settings-language-menu" class="settings-language-menu" role="listbox" aria-labelledby="settings-language-label" hidden>
                  <button class="settings-language-option" type="button" role="option" aria-selected="${draft.locale === "zh-CN"}" data-locale="zh-CN" lang="zh-CN"><span>${t("language.chinese")}</span><span class="language-selected-mark" aria-hidden="true"></span></button>
                  <button class="settings-language-option" type="button" role="option" aria-selected="${draft.locale === "en"}" data-locale="en" lang="en"><span>${t("language.english")}</span><span class="language-selected-mark" aria-hidden="true"></span></button>
                </div>
              </div>
            </div>
            <div class="setting-row settings-log-row">
              <div>
                <label>${t("settings.logTitle")}</label>
                <p>${t("settings.logDescription")}</p>
              </div>
              <button id="view-log" class="settings-action-button" type="button" ${state.busy ? "disabled" : ""}>${icon("file")}<span>${t("actions.viewLog")}</span></button>
            </div>
          </div>

          ${error}
          <footer class="settings-footer">
            <button id="settings-cancel" class="secondary-button" type="button" ${state.busy ? "disabled" : ""}>${t("settings.cancel")}</button>
            <button class="primary-button" type="submit" ${state.busy ? "disabled" : ""}>${state.busy ? t("settings.saving") : t("settings.save")}</button>
          </footer>
        </form>
      </section>
    </div>`;
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

  const settingsLabel = escapeHTML(t("settings.openAria"));

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
          <div class="top-retry-control ${snapshot.auto_retry_enabled ? "on" : "off"}">
            <span class="retry-state">${snapshot.auto_retry_enabled ? t("retry.on") : t("retry.off")}</span>
            <button id="auto-retry" class="toggle-switch ${snapshot.auto_retry_enabled ? "on" : "off"}" type="button" aria-label="${snapshot.auto_retry_enabled ? t("retry.disableAria") : t("retry.enableAria")}" aria-pressed="${snapshot.auto_retry_enabled}" ${state.busy ? "disabled" : ""}>
              <span></span>
            </button>
          </div>
          <button id="open-settings" class="utility-button" type="button" aria-label="${settingsLabel}" data-tooltip="${settingsLabel}" ${state.busy ? "disabled" : ""}>${icon("settings")}</button>
        </div>
      </header>

      <section class="overview-grid" aria-label="${t("overview.ariaLabel")}">
        <article class="overview-card current-card tone-${tone}">
          <div class="overview-heading">
            <span class="overview-kicker"><span class="pulse-mark"></span>${t("overview.currentStatus")}</span>
          </div>
          <h2>${escapeHTML(statusText(snapshot.status, snapshot.automatic_chain_limit))}</h2>
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

    </main>
    ${settingsDialog(snapshot)}`;

  restoreViewState(viewState);

  const autoRetry = document.querySelector<HTMLButtonElement>("#auto-retry");
  autoRetry?.addEventListener("click", async () => {
    state.busy = true;
    autoRetry.disabled = true;
    try {
      await invoke("set_auto_retry", { enabled: !snapshot.auto_retry_enabled });
      await loadSnapshot();
    } catch (error) {
      console.error(error);
    } finally {
      state.busy = false;
      render();
    }
  });

  document.querySelector<HTMLButtonElement>("#open-settings")?.addEventListener("click", () => {
    openSettings(snapshot);
  });

  document.querySelector<HTMLButtonElement>("#settings-close")?.addEventListener("click", () => {
    closeSettings();
  });
  document.querySelector<HTMLButtonElement>("#settings-cancel")?.addEventListener("click", () => {
    closeSettings();
  });
  document.querySelector<HTMLElement>("[data-settings-backdrop]")?.addEventListener("click", (event) => {
    if (event.target === event.currentTarget) closeSettings();
  });

  document.querySelector<HTMLInputElement>("#settings-chain-limit")?.addEventListener("input", (event) => {
    if (state.settingsDraft && event.currentTarget instanceof HTMLInputElement) {
      state.settingsDraft.automaticChainLimit = event.currentTarget.value;
    }
  });

  const retryMessage = document.querySelector<HTMLTextAreaElement>("#settings-retry-message");
  retryMessage?.addEventListener("input", () => {
    const draft = state.settingsDraft;
    if (!draft) return;
    draft.retryMessage = retryMessage.value;
    const count = [...retryMessage.value].length;
    const countLabel = document.querySelector<HTMLElement>("#settings-message-count");
    if (countLabel) {
      countLabel.textContent = t("settings.messageCount", {
        count,
        max: snapshot.retry_message_max_chars,
      });
      countLabel.classList.toggle("over-limit", count > snapshot.retry_message_max_chars);
    }
  });

  document.querySelector<HTMLButtonElement>("#settings-reset-message")?.addEventListener("click", () => {
    const draft = state.settingsDraft;
    if (!draft || !retryMessage) return;
    draft.retryMessage = snapshot.default_retry_message;
    retryMessage.value = snapshot.default_retry_message;
    retryMessage.dispatchEvent(new Event("input"));
    retryMessage.focus();
  });

  const settingsLanguageToggle = document.querySelector<HTMLButtonElement>("#settings-language-toggle");
  const settingsLanguageMenu = document.querySelector<HTMLElement>("#settings-language-menu");
  settingsLanguageToggle?.addEventListener("click", (event) => {
    event.stopPropagation();
    if (!settingsLanguageMenu) return;
    const willOpen = settingsLanguageMenu.hidden;
    settingsLanguageMenu.hidden = !willOpen;
    settingsLanguageToggle.setAttribute("aria-expanded", String(willOpen));
    if (willOpen) {
      settingsLanguageMenu
        .querySelector<HTMLButtonElement>('[aria-selected="true"]')
        ?.focus();
    }
  });
  document.querySelectorAll<HTMLButtonElement>(".settings-language-option").forEach((button) => {
    button.addEventListener("click", () => {
      const draft = state.settingsDraft;
      const locale = button.dataset.locale;
      if (!draft || (locale !== "zh-CN" && locale !== "en")) return;
      draft.locale = locale;
      document.querySelectorAll<HTMLButtonElement>(".settings-language-option").forEach((option) => {
        option.setAttribute("aria-selected", String(option.dataset.locale === locale));
      });
      const value = document.querySelector<HTMLElement>("#settings-language-value");
      if (value) value.textContent = locale === "zh-CN" ? t("language.chinese") : t("language.english");
      closeSettingsLanguageMenu();
      settingsLanguageToggle?.focus();
    });
  });

  document.querySelector<HTMLFormElement>("#settings-form")?.addEventListener("submit", (event) => {
    event.preventDefault();
    void saveSettings(snapshot);
  });

  const viewLog = document.querySelector<HTMLButtonElement>("#view-log");
  viewLog?.addEventListener("click", async () => {
    viewLog.disabled = true;
    try {
      await invoke("open_log");
    } catch (error) {
      console.error(error);
      viewLog.classList.add("error");
      const label = viewLog.querySelector<HTMLSpanElement>("span");
      if (label) label.textContent = t("actions.openFailed");
      window.setTimeout(() => {
        if (!viewLog.isConnected) return;
        viewLog.classList.remove("error");
        if (label) label.textContent = t("actions.viewLog");
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

function openSettings(snapshot: ContinuationSnapshot): void {
  if (state.settingsOpen) return;
  state.settingsOpen = true;
  state.settingsError = "";
  state.settingsDraft = {
    automaticChainLimit: String(snapshot.automatic_chain_limit),
    retryMessage: snapshot.retry_message,
    locale: getLocale(),
  };
  render();
  window.requestAnimationFrame(() => {
    document.querySelector<HTMLButtonElement>("#settings-close")?.focus();
  });
}

function closeSettings(): void {
  if (!state.settingsOpen || state.busy) return;
  state.settingsOpen = false;
  state.settingsDraft = null;
  state.settingsError = "";
  render();
  window.requestAnimationFrame(() => {
    document.querySelector<HTMLButtonElement>("#open-settings")?.focus();
  });
}

function closeSettingsLanguageMenu(): boolean {
  const menu = document.querySelector<HTMLElement>("#settings-language-menu");
  const toggle = document.querySelector<HTMLButtonElement>("#settings-language-toggle");
  if (!menu || menu.hidden) return false;
  menu.hidden = true;
  toggle?.setAttribute("aria-expanded", "false");
  return true;
}

async function saveSettings(snapshot: ContinuationSnapshot): Promise<void> {
  const draft = state.settingsDraft;
  if (!draft || state.busy) return;

  const automaticChainLimit = Number(draft.automaticChainLimit);
  if (
    !Number.isInteger(automaticChainLimit) ||
    automaticChainLimit < snapshot.automatic_chain_limit_min ||
    automaticChainLimit > snapshot.automatic_chain_limit_max
  ) {
    state.settingsError = t("settings.error.invalidLimit", {
      min: snapshot.automatic_chain_limit_min,
      max: snapshot.automatic_chain_limit_max,
    });
    render();
    window.requestAnimationFrame(() => {
      document.querySelector<HTMLInputElement>("#settings-chain-limit")?.focus();
    });
    return;
  }

  const retryMessage = draft.retryMessage.trim();
  if (!retryMessage) {
    state.settingsError = t("settings.error.emptyMessage");
    render();
    window.requestAnimationFrame(() => {
      document.querySelector<HTMLTextAreaElement>("#settings-retry-message")?.focus();
    });
    return;
  }
  if ([...retryMessage].length > snapshot.retry_message_max_chars) {
    state.settingsError = t("settings.error.longMessage", {
      max: snapshot.retry_message_max_chars,
    });
    render();
    window.requestAnimationFrame(() => {
      document.querySelector<HTMLTextAreaElement>("#settings-retry-message")?.focus();
    });
    return;
  }

  draft.retryMessage = retryMessage;
  state.settingsError = "";
  state.busy = true;
  render();
  try {
    await invoke("set_continuation_settings", {
      automaticChainLimit,
      retryMessage,
    });
    if (draft.locale !== getLocale()) {
      setLocale(draft.locale);
      await invoke("set_locale", { locale: draft.locale }).catch((error) => console.error(error));
    }
    await loadSnapshot();
    state.busy = false;
    state.settingsOpen = false;
    state.settingsDraft = null;
    render();
    window.requestAnimationFrame(() => {
      document.querySelector<HTMLButtonElement>("#open-settings")?.focus();
    });
  } catch (error) {
    console.error(error);
    state.busy = false;
    state.settingsError = t("settings.error.saveFailed");
    render();
  }
}

document.addEventListener("keydown", (event) => {
  if (!state.settingsOpen) return;
  if (event.key === "Escape") {
    event.preventDefault();
    if (closeSettingsLanguageMenu()) {
      document.querySelector<HTMLButtonElement>("#settings-language-toggle")?.focus();
      return;
    }
    closeSettings();
    return;
  }
  if (event.key !== "Tab") return;
  const dialog = document.querySelector<HTMLElement>(".settings-dialog");
  if (!dialog) return;
  const focusable = Array.from(
    dialog.querySelectorAll<HTMLElement>(
      'button:not(:disabled), input:not(:disabled), textarea:not(:disabled), select:not(:disabled)',
    ),
  );
  if (!focusable.length) return;
  const first = focusable[0];
  const last = focusable[focusable.length - 1];
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault();
    first.focus();
  }
});

document.addEventListener("click", (event) => {
  if (!state.settingsOpen || !(event.target instanceof Node)) return;
  const picker = document.querySelector<HTMLElement>(".settings-language-picker");
  if (picker?.contains(event.target)) return;
  closeSettingsLanguageMenu();
});

async function loadSnapshot(): Promise<void> {
  try {
    const snapshot = await invoke<ContinuationSnapshot>("get_snapshot");
    const signature = JSON.stringify(snapshot);
    if (signature === state.snapshotSignature) return;
    state.snapshot = snapshot;
    state.snapshotSignature = signature;
    if (state.settingsOpen) return;
    render();
  } catch (error) {
    console.error(error);
  }
}

async function openSettingsFromTray(): Promise<void> {
  if (state.settingsOpen) return;
  if (!state.snapshot) await loadSnapshot();
  if (state.snapshot) openSettings(state.snapshot);
}

render();
void listen("open-settings", () => {
  void openSettingsFromTray();
}).catch((error) => console.error(error));
void invoke("set_locale", { locale: getLocale() }).catch((error) => console.error(error));
void loadSnapshot();
window.setInterval(() => void loadSnapshot(), 3000);
