# Agent Mission Control

Agent Mission Control 是一款面向 Windows 的本地优先桌面客户端，用飞船驾驶舱式界面呈现和控制 Coding Agent 任务。它负责统一展示任务状态、执行航线、审批、预算、证据与恢复信息，并通过独立 Supervisor 约束本地 Agent 进程。

## 技术栈

- Tauri 2、Rust 和 Windows 原生 IPC
- React 19、TypeScript、Vite 和 Three.js
- SQLite/SQLCipher 事件账本
- Vitest、Playwright、Cargo Test 和 Pester

## 目录

- `apps/desktop/`：Tauri 桌面客户端和 React 驾驶舱界面
- `crates/`：领域模型、策略、协议、账本、工作区和 Supervisor
- `packages/`：前端 UI、协议绑定和任务状态存储
- `fixtures/`：协议、恢复和安全测试数据
- `scripts/`：环境检查、集成测试和打包验证脚本
- `docs/development/`：开发环境说明

## 开发

先按 [Windows development setup](docs/development/windows-setup.md) 安装 Node.js、Rust、Visual Studio Build Tools、Windows SDK 和 WebView2 Runtime，然后执行：

```powershell
npm ci
npm run verify:workspace
npm run tauri:dev
```

构建 MSI：

```powershell
npm run tauri:build
```

运行主要验证：

```powershell
npm test
cargo test --workspace --locked
npm run test:integration
npm run test:visual
```

当前版本为 `0.1.0` 技术预览版。
