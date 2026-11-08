# 开发与发布检查

## 在 Windows 上构建

安装 Visual Studio C++ Build Tools、Windows SDK 和 CMake。Rust 版本由仓库的 `rust-toolchain.toml` 固定。

```powershell
cargo run --locked
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Windows CI 运行格式检查、Clippy 和默认测试。需要桌面、真实内核或特定数据的测试由各自的忽略标记说明前置条件。界面和任务栏用例可通过独立入口运行，日志位于 `target/desktop-tests`。

```powershell
.\scripts\check-desktop.ps1 -Suite Ui
.\scripts\check-desktop.ps1 -Suite Taskbar
.\scripts\check-desktop.ps1 -Suite Ui -List
```

`-Filter` 可按名称缩小范围，`-Release` 使用 Release 构建。任务栏检查需要正在运行的 Explorer。

## 代码入口

| 位置 | 职责 |
| --- | --- |
| `src/main.rs`、`src/backend/` | 启动、后台采集、查询和设置 |
| `src/mihomo.rs`、`src/accounting.rs` | 内核通信、计数器和流量归属 |
| `src/storage.rs`、`src/storage_management.rs` | SQLite 账本、恢复和保留策略 |
| `src/ui/`、`src/native/` | 桌面界面和 Windows 集成 |

## 打包与发布前验证

运行 `.\scripts\package.ps1` 生成安装包、便携 ZIP 和本地核对用的 SHA-256 文件。打包构建会重映射并检查本机源码路径。安装包需要 Inno Setup 6.3 或更新版本。用 `-InnoCompiler` 指定编译器，`-ArchiveOnly` 仅生成 ZIP，`-SkipBuild` 复用前一次打包构建。

1. 通过默认检查和受影响的桌面回归。使用独立数据目录验证真实 Mihomo 连接、断线重连、基线和流量缺口。
2. 验证旧账本恢复、备份、目录迁移和保留策略。检查任务栏与悬浮窗在系统主题、Explorer 重启、全屏、多显示器和 DPI 切换时的表现。
3. 测试安装版和便携版的首次运行与升级，检查包内第三方声明和本地 SHA-256 校验文件。
4. 复核 Git 历史、截图和发布包。仓库截图使用合成数据。发布包包含程序、许可证和第三方声明，便携包另含 `portable.flag`。

发布正式版时用 `v` 加版本号作为标签，例如 `v0.1.0`。把更新说明写入 Release 正文，按原文件名上传安装包和便携 ZIP，并标记为最新版。应用从同一份 Release 读取说明和下载入口。

可用 `mihomo-traffic.exe --probe` 读取当前内核状态，或用 `--headless --seconds 10 --data-dir <独立目录>` 采集一段数据。
