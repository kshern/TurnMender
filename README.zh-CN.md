<p align="center">
  <img src="assets/turnmender-logo.png" alt="TurnMender Logo" width="176" />
</p>

<h1 align="center">TurnMender</h1>

<p align="center">
  <a href="README.md">English</a> · <strong>简体中文</strong>
</p>

TurnMender 用来解决 Codex 任务因模型容量不足而中断的问题：

```text
Selected model is at capacity
Error running remote compact task: Selected model is at capacity
```

遇到这类报错时，TurnMender 会确认当前轮次没有最终回复，然后在**原任务**中自动继续。普通任务会收到继续指令；目标模式中的目标如果被报错暂停，则会自动恢复为运行状态。

## 下载

前往 [GitHub Releases](https://github.com/kshern/TurnMender/releases/latest) 下载最新版。目前支持 macOS 和 Windows。

## 使用方法

1. 保持 Codex 桌面端运行。
2. 安装并打开 TurnMender。
3. 保持“自动继续”开启即可。

关闭主窗口后，TurnMender 仍会在系统托盘中运行。你可以在设置中修改继续指令和单个任务的连续自动继续次数；默认上限为 50 次。

## 它会做什么

- 在本机监听 Codex 任务记录，只处理模型容量不足的报错。
- 仅在失败轮次没有最终回复时继续，避免重复发送。
- 目标模式因容量错误暂停时，自动恢复原目标；状态切换期间不会额外发送普通续行消息，也不会改动已阻塞、额度受限或已完成的目标。
- 按任务 ID 回到原任务，并记录已处理事件，避免循环触发。
- 不需要 OpenAI API Key，不上传任务内容，也不操作键盘、鼠标或剪贴板。

> TurnMender 是非官方工具，与 OpenAI 没有关联。它依赖 Codex 桌面端当前的本地消息通道；Codex 更新后，兼容性可能暂时受到影响。

## 本地开发

需要 Node.js 20+、pnpm 10、Rust 1.77.2+ 和对应平台的 Tauri 构建环境。

```sh
pnpm install
pnpm tauri dev
```

构建安装包：

```sh
pnpm tauri build
```

更多实现细节见 [ARCHITECTURE.md](ARCHITECTURE.md)。项目采用 [MIT License](LICENSE)。
