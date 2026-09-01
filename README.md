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

## 从源码构建

先按 [Windows development setup](docs/development/windows-setup.md) 安装 Node.js、Rust、Visual Studio Build Tools、Windows SDK 和 WebView2 Runtime。SQLCipher 使用 vendored OpenSSL，Windows 构建还需要完整的 Perl 环境（推荐 Strawberry Perl）并正确加入 `PATH`。

安装依赖并启动开发版：

```powershell
npm ci
npm run verify:workspace
npm run tauri:dev
```

构建 MSI：

```powershell
npm run tauri:build
```

## 开发工作流

提交前运行格式化、静态检查、全量 Rust 测试以及前端类型检查和 Vitest：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline
npm run typecheck
npm test
```

发布门禁会额外运行安全、恢复、soak、协议一致性、SBOM 和证据校验：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-codex-release.ps1 -Distribution oss-personal
```

未提供真实调用证据时，命令仍会诚实地保持 `releaseReady=false`；不要用 fixture 或 dry run 改标。

## Windows 未签名构建

个人开源分发允许发布未签名 MSI。Windows SmartScreen 可能显示“Windows 已保护你的电脑”；确认制品 SHA-256 与该 GitHub Release 公布值一致后，可选择“更多信息”再选择“仍要运行”。这只是绕过信誉提示，不等于系统替你验证了发布者。对来源或哈希有疑问时应取消安装并从源码构建。

代码签名是推荐的增强项，不是 `oss-personal` 模式的阻断项。可选路径见 [正式发布手册](docs/release/formal-release-runbook.md)。

## 安全与隐私

安全问题请按 [SECURITY.md](SECURITY.md) 使用私密渠道报告。不要在公开 issue 中粘贴凭据、原始 Provider 事件或用户源码。

## License 与免责声明

本项目按 [MIT License](LICENSE) 发布。软件按“原样”提供，不作任何明示或暗示担保（disclaimer）；使用本地 Agent、执行命令和安装未签名构建所产生的风险由使用者自行评估。

## 版本状态

当前版本为 `0.1.0` 技术预览版。
