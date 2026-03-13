# 方案三落地清单

目标：在 Tauri 安装包内分发完整 Node 运行时与 OpenClaw npm 包，应用优先使用内置运行时；缺失时回退到现有系统全局安装模式。

## 目录布局

构建产物统一放到 `src-tauri/runtime/`：

```text
src-tauri/runtime/
├── tools/
│   └── node/                  # 完整 Node 运行时目录，需包含 npm
├── lib/
│   └── node_modules/
│       └── openclaw/          # 完整 OpenClaw npm 包目录
└── manifest.json              # 生成清单，仅保留版本与相对布局信息
```

## 仓库改造

1. Tauri `bundle.resources` 打入 `runtime/**`。
2. Rust 启动时记录 `resource_dir`，统一解析内置运行时目录。
3. `shell` 层优先使用内置 Node + OpenClaw 入口；找不到再回退全局 `openclaw`。
4. 环境检查、系统信息、诊断页显示当前运行时模式。
5. 安装、卸载、更新命令在内置运行时模式下返回“不可用/无需安装”。

## 构建流程

1. 在目标平台准备 Node 22+/24+ 构建环境。
2. macOS 需要分别产出 `x86_64-apple-darwin` 与 `aarch64-apple-darwin` 安装包，不能继续依赖单个 universal 包复用同一份 Node/OpenClaw runtime。
3. Windows 产物建议显式启用离线 WebView2 安装器，避免首次安装依赖外网下载。
4. 执行 `npm run prepare:bundled-runtime`。
5. 该脚本会：
   - 复制当前 Node 运行时目录到 `src-tauri/runtime/tools/node`
   - 用 `npm install -g --prefix <staging>` 安装指定版本 OpenClaw
   - 复制完整 OpenClaw 包目录到 `src-tauri/runtime/lib/node_modules/openclaw`
   - 生成 `manifest.json`，避免写入构建机绝对路径
6. 随后执行 `npm run tauri:build` 打包。
7. 构建完成后执行 `npm run verify:bundled-runtime -- --target <triple> --platform <os>`，确认 `target/.../release/runtime` 已带上 Node 与 OpenClaw 入口；macOS 额外检查 `.app/Contents/Resources/runtime`。

## 运行时策略

1. 优先探测 `src-tauri/runtime` 或安装包资源目录中的内置运行时。
2. 内置运行时启动命令为：`<bundled-node> <openclaw-entry> ...args`
3. 不显式注入 `OPENCLAW_HOME`，避免与 OpenClaw 自身的路径解析规则冲突。
4. BosiClaw 自己解析配置/日志路径时，优先尊重 `OPENCLAW_CONFIG_PATH`、`OPENCLAW_STATE_DIR`、`OPENCLAW_HOME`。
5. `PATH` 首部加入内置 Node 的 bin 目录，保证 OpenClaw 运行期可调用 `node` / `npm`。

## 已知限制

1. 内置模式下不支持应用内 `npm install -g openclaw`、`npm uninstall -g openclaw`、`npm install -g openclaw@latest`。
2. 原生 Windows 路径仍需重点验证；上游对 Windows 仍偏向 WSL2。
3. OpenClaw 依赖的原生模块必须按目标平台单独构建，不能跨平台复用 `node_modules`。
4. 如果未来要做“应用内更新 OpenClaw”，应改成“重新准备 runtime 并发布新安装包”，而不是在已安装包内原地更新。

## 建议验证项

1. `check_environment` 能识别 `bundled` 模式。
2. `openclaw --version`、`openclaw doctor`、`openclaw gateway --port 18789` 能正常执行。
3. Dashboard 能加载，日志能写入 `~/.openclaw/logs`。
4. `plugins install` 仍能写入用户状态目录，不依赖安装包可写。
