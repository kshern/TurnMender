<p align="center">
  <img src="assets/turnmender-logo.png" alt="TurnMender logo" width="176" />
</p>

<h1 align="center">TurnMender</h1>

<p align="center">
  <strong>English</strong> · <a href="README.zh-CN.md">简体中文</a>
</p>

TurnMender fixes Codex tasks interrupted by model-capacity errors:

```text
Selected model is at capacity
Error running remote compact task: Selected model is at capacity
```

When either error occurs, TurnMender confirms that the failed turn has no final response, then continues the **original task** automatically. Normal tasks receive a continuation instruction; if Goal mode was paused by the error, TurnMender resumes that goal instead.

## Download

Get the latest macOS or Windows build from [GitHub Releases](https://github.com/kshern/TurnMender/releases/latest).

## Usage

1. Keep Codex desktop running.
2. Install and open TurnMender.
3. Leave **Automatic continuation** enabled.

TurnMender keeps running in the system tray after its main window is closed. In Settings, you can change the continuation instruction and the consecutive continuation limit for each task. The default limit is 50.

## What it does

- Watches local Codex task records and handles only model-capacity errors.
- Continues only when the failed turn has no final response, preventing duplicate messages.
- Resumes a Goal-mode goal paused by a capacity error without sending a duplicate normal continuation during the state transition, and without changing blocked, usage-limited, budget-limited, or completed goals.
- Returns to the original task by task ID and remembers handled events to prevent loops.
- Requires no OpenAI API key, uploads no task content, and does not control the keyboard, mouse, or clipboard.

> TurnMender is an unofficial project and is not affiliated with OpenAI. It relies on Codex desktop's current local messaging channel, so a future Codex update may temporarily affect compatibility.

## Local development

Requires Node.js 20+, pnpm 10, Rust 1.77.2+, and the Tauri prerequisites for your platform.

```sh
pnpm install
pnpm tauri dev
```

Build an installer:

```sh
pnpm tauri build
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for implementation details. Licensed under the [MIT License](LICENSE).
