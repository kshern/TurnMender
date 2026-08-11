<p align="center">
  <img src="assets/turnmender-logo.png" alt="TurnMender logo" width="176" />
</p>

<h1 align="center">TurnMender</h1>

<p align="center">
  <strong>English</strong> · <a href="README.zh-CN.md">简体中文</a>
</p>

TurnMender is a local continuation helper built for Codex desktop. It watches the local records of every Codex task. When a turn clearly stops because the selected model is at capacity and no final response was produced, TurnMender attempts to start another turn in the original task so the work can continue.

It does not require an OpenAI API key, upload task content, or interact with the keyboard, mouse, clipboard, or windows. TurnMender is an independent, unofficial personal project and is not affiliated with OpenAI.

> TurnMender is built with Tauri 2. Automatic continuation is available on macOS. Windows currently supports status monitoring only and does not send continuation messages.

> [!IMPORTANT]
> Each task automatically continues up to **50 consecutive times** by default. You can change this limit to any whole number of at least 1 in **Settings**.

## Download

Download the latest build from [GitHub Releases](https://github.com/kshern/TurnMender/releases/latest). The current DMG is built for Apple Silicon Macs.

## Features

- Incrementally watches all Codex task records under `~/.codex/sessions`.
- Handles only explicit model-capacity errors and leaves ordinary failures untouched.
- Confirms that the failed turn has no final response before continuing, preventing duplicate messages after a task has already completed.
- Sends the continuation message back to the original task by task ID instead of guessing from the active window or task title.
- Shows the overall continuation status, recent tasks, task names, last activity times, and cases that require manual action.
- Lets you pause or resume automatic continuation at any time while monitoring remains active.
- Lets you set the consecutive automatic-continuation limit to any whole number of at least 1 (50 by default).
- Provides a refresh icon beside each task's continuation count to clear that task's consecutive count.
- Lets you customize the instruction sent to the original task and restore the default at any time.
- Uses colored tray icons for healthy, attention, and action-required states.
- Keeps running after the main window is closed; the tray menu shows status, toggles automatic continuation, opens tasks needing attention, provides Settings and log shortcuts, and can quit completely.
- Persists deduplication records and consecutive continuation counts so the same error is not handled again after a restart.
- Maintains a local runtime log that can be opened directly from the main window.

## Platform support

| Platform | Monitoring and UI | Automatic continuation | Current status |
| --- | --- | --- | --- |
| macOS | Supported | Supported | Uses the local messaging channel provided by Codex desktop |
| Windows | Shared logic implemented; hardware validation pending | Not yet supported | Monitors and asks for manual continuation until a suitable channel is confirmed |
| Linux / WSL2 | Not officially supported | Not supported | Outside the current release targets |

Automatic continuation requires Codex desktop to be running and its local messaging channel to be available. If the channel is missing, the protocol changes, the original task cannot be found, or no explicit acknowledgement is received, TurnMender stops automatic handling for that event and asks for manual continuation.

## Decision rules

TurnMender makes a decision only after reading a complete `task_complete` record. It never acts on text that is still being generated.

| Record state | Action |
| --- | --- |
| Explicit capacity error with an empty `last_agent_message` | Marks the task as ready to continue; continues automatically when enabled and the channel is available |
| Explicit capacity error with a final response | Marks the task as already answered and sends nothing |
| Explicit capacity error missing required decision fields | Marks the task for review and sends nothing automatically |
| Ordinary network, permission, or application error | Ignores the error |
| The user manually starts another turn after the error | Cancels the old event and does not send a duplicate message |

Additional safeguards prevent loops and accidental triggers:

- Deduplication uses the task ID together with the failed turn ID.
- A continuation counts as successful only after an acknowledgement containing a new turn ID is received.
- When a task reaches the configured consecutive automatic-continuation limit, TurnMender switches to manual handling. The limit can be set to any whole number of at least 1 in Settings and defaults to 50.
- A manually sent message clears the consecutive count for that task.
- At startup, TurnMender checks only records updated within the last 10 minutes and does not process much older failures.
- If sending fails, it does not fall back to command-line or UI automation.

## Interface and tray states

The main window refreshes every three seconds and shows up to 100 recently recognized tasks. A task may appear as running, ready to continue, already answered, idle, requiring review, or channel unavailable. Removing a task from the recent list does not delete its Codex session; it will reappear when new activity occurs.

The tray and main window use the same status colors:

- Green: monitoring and messaging are healthy, with no pending exceptions.
- Yellow: automatic continuation is paused, the messaging channel is not ready, or a task is waiting.
- Red: monitoring failed, or a task requires manual review or continuation.

The tray menu explains the current state and includes a checked automatic-continuation toggle. When tasks need attention, it lists the three most recent tasks and opens the selected task directly in Codex; the main window contains the full list. Settings and the runtime log are also available from the tray. The window header keeps a second quick toggle, while the chain limit, continuation instruction, and interface language stay in Settings. Closing the main window only hides it; use **Quit TurnMender** from the tray menu to stop it completely.

## Local development

### Prerequisites

- Rust 1.77.2 or later
- Node.js 20 or later
- pnpm 10
- The Tauri build prerequisites for your platform

Automatic continuation on macOS also requires Codex desktop to remain running.

### Run the development build

```sh
pnpm install
pnpm tauri dev
```

Automatic continuation is enabled by default. When a matching capacity error is found, the development build may send a real continuation message on macOS. Check the toggle in the main window before debugging.

### Checks and tests

Check the frontend:

```sh
pnpm build
```

Test and check the Rust backend:

```sh
cd src-tauri
cargo test --all-targets
cargo check --all-targets
```

### Build an installer

From the project root:

```sh
pnpm tauri build
```

To build only a debug executable without an installer:

```sh
pnpm tauri build --debug --no-bundle
```

## Local data and privacy

The Codex task directory is resolved from `CODEX_HOME` when set, otherwise from `~/.codex`:

```text
Task records: ~/.codex/sessions
macOS messaging channel: ~/.codex/ipc/ipc.sock
```

When `CODEX_HOME` is set, these paths become `$CODEX_HOME/sessions` and `$CODEX_HOME/ipc/ipc.sock`.

Application state, settings, and logs are stored in the TurnMender local data directory:

| Platform | Local data directory |
| --- | --- |
| macOS | `~/Library/Application Support/TurnMender` |
| Windows | `%LOCALAPPDATA%\TurnMender` |

`state.json` stores deduplication data and consecutive counts, `config.json` stores the automatic-continuation switch, chain limit, and custom continuation instruction, and `turnmender.log` stores runtime logs.

All decisions are made locally. State files do not store task bodies. Logs contain only task IDs, turn IDs, important automatic-continuation results, and errors. The main window and tray show the live monitoring state, so heartbeat entries are not periodically written to the log. Logs rotate after reaching 5 MB, keeping one older file with a `.1` suffix.

## Security boundaries

The current Tauri implementation explicitly does not:

- Run `codex exec resume` or start other Codex CLI subprocesses.
- Request macOS Accessibility permission.
- Simulate keyboard, mouse, focus, clipboard, or window actions.
- Guess a target from the task title or active foreground window.
- Switch to another sending method when the local messaging channel fails.
- Automatically change the model or account, or handle errors other than model capacity.

## Project structure

```text
TurnMender/
├── assets/
│   └── turnmender-logo.png         # Standalone transparent project logo
├── src/
│   ├── main.ts                     # Main UI, status refresh, and interactions
│   ├── i18n.ts                     # UI copy and language switching
│   └── styles.css                  # UI styles
├── src-tauri/
│   ├── src/core/                   # Error classification, task state, and deduplication policy
│   ├── src/watcher/                # Incremental JSONL task-record watcher
│   ├── src/transport/              # Sends continuation messages by task ID
│   ├── src/service.rs              # Continuation workflow and consecutive-run protection
│   ├── src/storage.rs              # Settings, state, and logs
│   └── tauri.conf.json             # Desktop application and bundle configuration
├── ARCHITECTURE.md                 # Architecture and security boundaries
├── LICENSE                         # MIT License
├── README.md                       # English documentation
└── README.zh-CN.md                 # Simplified Chinese documentation
```

## Current limitations

- TurnMender depends on the current Codex local task-record format. Parser updates may be required when that format changes.
- Automatic continuation on macOS uses a local versioned channel rather than a public stable API. A Codex update may temporarily break compatibility.
- Windows does not yet have a local channel that can target a task by ID and return an explicit acknowledgement.
- Only model-capacity errors are handled. TurnMender does not process other errors or switch models automatically.
- Startup recovery checks only the most recent 10 minutes; older historical errors are not processed automatically.
- The project remains under active development and validation.

See [ARCHITECTURE.md](ARCHITECTURE.md) for more details about the design and security boundaries.

## License

TurnMender is available under the [MIT License](LICENSE). You may use, modify, and distribute it as long as the copyright and license notices are preserved.
