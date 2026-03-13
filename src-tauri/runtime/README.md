# Bundled Runtime

这个目录用于存放随 Tauri 安装包一起分发的 OpenClaw 运行时资源。

期望布局：

```text
runtime/
├── tools/
│   └── node/                  # 完整 Node 运行时目录，需包含 npm
└── lib/
    └── node_modules/
        └── openclaw/          # 完整 OpenClaw npm 包目录
```

请不要手动把开发机上的任意目录直接复制到这里。优先使用仓库根目录下的：

```bash
npm run prepare:bundled-runtime
```

该脚本会按当前平台准备运行时，并写入此目录。实际二进制与依赖文件已被 `.gitignore` 忽略，不应提交到仓库。
