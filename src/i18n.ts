export type Locale = "zh-CN" | "en";

const STORAGE_KEY = "turnmender.locale";

const zhCN = {
  "meta.description": "TurnMender Codex 容量中断续行工具",
  "brand.subtitle": "Codex 容量中断续行",
  "language.change": "切换语言",
  "language.chinese": "中文",
  "language.english": "English",
  "actions.viewLog": "显示日志",
  "actions.openFailed": "打开失败",
  "loading.snapshot": "正在读取续行状态…",
  "time.noActivity": "暂无活动",
  "task.unnamed": "未命名任务",
  "task.idTitle": "任务 ID：{id}",
  "task.projectUnknown": "未知项目",
  "task.projectPathTitle": "项目路径：{path}",
  "task.open": "打开",
  "task.openAria": "在 Codex 中打开“{task}”",
  "task.dismiss": "从最近任务中移除",
  "task.dismissAria": "从最近任务中移除“{task}”",
  "task.state.running": "运行中",
  "task.state.waitingContinuation": "待继续",
  "task.state.completedWithOutput": "已有回复",
  "task.state.idle": "空闲",
  "task.state.unknown": "需确认",
  "task.state.unavailable": "通道不可用",
  "task.description.waitingContinuation": "容量错误已记录",
  "task.description.unknown": "自动处理结果待确认",
  "task.description.unavailable": "需要手动继续",
  "task.description.lastActivity": "最后活动",
  "task.continuationCount": "自动继续 {count} 次",
  "task.continuationCountTitle": "当前连续自动继续次数",
  "channel.ready": "消息通道正常",
  "channel.unavailable": "消息通道不可用",
  "channel.unsupported": "当前平台不支持",
  "channel.unknown": "通道状态未知",
  "health.stopped": "已停止",
  "health.actionRequired": "需要处理",
  "health.monitorOnly": "仅监听",
  "health.needsAttention": "需要留意",
  "health.healthy": "续行就绪",
  "health.description.stopped": "自动续行已暂停，任务不会自动继续。",
  "health.description.actionRequired": "检测到需要人工确认的任务或监听异常。",
  "health.description.monitorOnly": "监听仍在运行，但自动继续已暂停。",
  "health.description.channel": "监听仍在运行，但消息通道需要检查。",
  "health.description.waiting": "有任务正在等待后续处理。",
  "health.description.healthy": "发现容量错误后会自动继续。",
  "overview.ariaLabel": "续行概览",
  "overview.currentStatus": "当前状态",
  "overview.tasks": "个任务",
  "overview.running": "运行中",
  "overview.needsAttention": "需处理",
  "retry.title": "自动继续",
  "retry.on": "已开启",
  "retry.off": "已暂停",
  "retry.disableAria": "关闭自动继续",
  "retry.enableAria": "开启自动继续",
  "retry.valueOn": "容量错误时继续原任务",
  "retry.valueOff": "当前仅监听任务状态",
  "retry.description": "只处理明确的容量错误，普通失败不会操作。",
  "tasks.title": "最近任务",
  "tasks.autoUpdates": "状态自动更新",
  "empty.title": "一切安静",
  "empty.description": "暂时没有发现任务记录",
  "status.preparing": "准备监听",
  "status.watchFailed": "监听失败：{detail}",
  "status.watching": "正在监听",
  "status.watchingUnsupported": "正在监听，当前平台暂不支持自动发送",
  "status.watchingChannelUnavailable": "正在监听，消息通道不可用",
  "status.taskWaiting": "「{task}」等待处理",
  "status.chainProtected": "「{task}」已触发连续保护",
  "status.manualContinue": "「{task}」需要人工继续",
  "status.continuing": "正在继续「{task}」",
  "status.continued": "已继续「{task}」",
  "status.confirmSend": "「{task}」需要人工确认发送结果",
  "status.stopped": "续行服务已停止",
  "status.unknown": "状态未知",
} as const;

type TranslationKey = keyof typeof zhCN;

const en = {
  "meta.description": "TurnMender capacity continuation for Codex tasks",
  "brand.subtitle": "Codex capacity continuation",
  "language.change": "Change language",
  "language.chinese": "中文",
  "language.english": "English",
  "actions.viewLog": "Show log",
  "actions.openFailed": "Could not open",
  "loading.snapshot": "Reading continuation status…",
  "time.noActivity": "No activity",
  "task.unnamed": "Unnamed task",
  "task.idTitle": "Task ID: {id}",
  "task.projectUnknown": "Unknown project",
  "task.projectPathTitle": "Project path: {path}",
  "task.open": "Open",
  "task.openAria": "Open “{task}” in Codex",
  "task.dismiss": "Remove from recent tasks",
  "task.dismissAria": "Remove “{task}” from recent tasks",
  "task.state.running": "Running",
  "task.state.waitingContinuation": "Waiting",
  "task.state.completedWithOutput": "Response received",
  "task.state.idle": "Idle",
  "task.state.unknown": "Needs review",
  "task.state.unavailable": "Channel unavailable",
  "task.description.waitingContinuation": "Capacity error recorded",
  "task.description.unknown": "Automatic handling needs confirmation",
  "task.description.unavailable": "Manual continuation required",
  "task.description.lastActivity": "Last activity",
  "task.continuationCount": "Automatic continuations: {count}",
  "task.continuationCountTitle": "Current consecutive automatic continuation count",
  "channel.ready": "Messaging ready",
  "channel.unavailable": "Messaging unavailable",
  "channel.unsupported": "Unsupported on this platform",
  "channel.unknown": "Messaging status unknown",
  "health.stopped": "Stopped",
  "health.actionRequired": "Action required",
  "health.monitorOnly": "Monitor only",
  "health.needsAttention": "Needs attention",
  "health.healthy": "Continuation ready",
  "health.description.stopped": "Automatic continuation is paused and tasks will not continue automatically.",
  "health.description.actionRequired": "A task or monitoring issue needs manual review.",
  "health.description.monitorOnly": "Monitoring is active, but automatic continuation is paused.",
  "health.description.channel": "Monitoring is active, but the messaging channel needs attention.",
  "health.description.waiting": "A task is waiting for follow-up handling.",
  "health.description.healthy": "Capacity errors will be continued automatically.",
  "overview.ariaLabel": "Continuation overview",
  "overview.currentStatus": "Current status",
  "overview.tasks": "tasks",
  "overview.running": "running",
  "overview.needsAttention": "need attention",
  "retry.title": "Auto-continue",
  "retry.on": "On",
  "retry.off": "Paused",
  "retry.disableAria": "Turn off automatic continuation",
  "retry.enableAria": "Turn on automatic continuation",
  "retry.valueOn": "Continue the original task after a capacity error",
  "retry.valueOff": "Monitoring task status only",
  "retry.description": "Only clear capacity errors are handled; ordinary failures are left untouched.",
  "tasks.title": "Recent tasks",
  "tasks.autoUpdates": "Status updates automatically",
  "empty.title": "All quiet",
  "empty.description": "No task activity has been found yet",
  "status.preparing": "Preparing to monitor",
  "status.watchFailed": "Monitoring failed: {detail}",
  "status.watching": "Monitoring",
  "status.watchingUnsupported": "Monitoring; automatic continuation is not supported on this platform",
  "status.watchingChannelUnavailable": "Monitoring; messaging is unavailable",
  "status.taskWaiting": "“{task}” is waiting for attention",
  "status.chainProtected": "“{task}” reached the automatic continuation limit",
  "status.manualContinue": "“{task}” needs manual continuation",
  "status.continuing": "Continuing “{task}”",
  "status.continued": "Continued “{task}”",
  "status.confirmSend": "“{task}” needs confirmation of the send result",
  "status.stopped": "Continuation stopped",
  "status.unknown": "Status unknown",
} satisfies Record<TranslationKey, string>;

const messages: Record<Locale, Record<TranslationKey, string>> = {
  "zh-CN": zhCN,
  en,
};

function normalizeLocale(value: string | null | undefined): Locale | null {
  if (!value) return null;
  const normalized = value.trim().toLowerCase();
  if (normalized.startsWith("zh")) return "zh-CN";
  if (normalized.startsWith("en")) return "en";
  return null;
}

function detectLocale(): Locale {
  try {
    const saved = normalizeLocale(window.localStorage.getItem(STORAGE_KEY));
    if (saved) return saved;
  } catch {
    // The system language fallback still works when storage is unavailable.
  }

  for (const language of navigator.languages ?? [navigator.language]) {
    const locale = normalizeLocale(language);
    if (locale) return locale;
  }
  return "en";
}

let currentLocale = detectLocale();

function syncDocumentLanguage(): void {
  document.documentElement.lang = currentLocale;
  const description = document.querySelector<HTMLMetaElement>('meta[name="description"]');
  if (description) description.content = t("meta.description");
}

export function getLocale(): Locale {
  return currentLocale;
}

export function setLocale(locale: Locale): void {
  currentLocale = locale;
  try {
    window.localStorage.setItem(STORAGE_KEY, locale);
  } catch {
    // Language switching still works for the current session without storage.
  }
  syncDocumentLanguage();
}

export function t(
  key: TranslationKey,
  values: Record<string, string | number> = {},
): string {
  return messages[currentLocale][key].replace(/\{(\w+)\}/g, (placeholder, name: string) =>
    Object.prototype.hasOwnProperty.call(values, name) ? String(values[name]) : placeholder,
  );
}

syncDocumentLanguage();
