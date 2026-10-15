# 开发工具位置

检查日期：2026-10-13。项目根目录：`D:\phiraiad\PhiraiAd`。当前仓库要求以其 `rust-toolchain.toml` 和构建脚本为准。

## Windows 本机可用工具

| 工具 | 位置 | 检查到的内容 |
|---|---|---|
| Git | `C:\Program Files\Git\cmd\git.exe` | Git 命令可用 |
| Java | `D:\bin\java.exe` | OpenJDK 17.0.13（Microsoft build） |
| Python | `D:\AgentReach\venv\Scripts\python.exe` | 当前 PATH 命中的 Python；这是 AgentReach 虚拟环境，不应当作项目专用环境 |
| Node.js | `C:\Program Files\nodejs\node.exe` | Node 命令可用 |
| VS Code CLI | `D:\Microsoft VS Code\bin\code.cmd` | `code` 命令可用 |
| Android SDK（Windows） | `C:\Users\Lenovo\AppData\Local\Android\Sdk` | `platform-tools`、`cmdline-tools\latest`、`platforms\android-37.2`、`build-tools\37.0.0` |

Windows Android SDK 与下方 WSL 项目 SDK 是两套独立目录。PhiraiAd 的 Linux/Android 构建脚本在 UbuntuD 中运行时，优先使用 WSL 环境变量指向的 D 盘 SDK。

## UbuntuD / WSL 项目构建工具

UbuntuD 的发行版文件系统位于 D 盘；从 UbuntuD 访问 Windows D 盘时路径前缀为 `/mnt/d`。

| 工具 | UbuntuD 路径 | 版本 / 备注 |
|---|---|---|
| Rust 安装器目录 | `/home/jouissance/.cargo` | `rustup` 和 `cargo` 用户级安装目录 |
| Rust toolchains | `/home/jouissance/.rustup` | 默认 `nightly-2026-09-14-x86_64-unknown-linux-gnu`；`rustc 1.100.0-nightly`、`cargo 1.100.0-nightly` |
| Rust Android target | Rustup target | 已安装 `aarch64-linux-android`；另有 `x86_64-unknown-linux-gnu` |
| Android SDK | `/mnt/d/Android/Sdk` | `ANDROID_HOME` 与 `ANDROID_SDK_ROOT` |
| Android NDK | `/mnt/d/Android/Sdk/ndk/27.3.13750724` | NDK r27d；`ANDROID_NDK_HOME` 与 `ANDROID_NDK_ROOT` |
| Android Build Tools | `/mnt/d/Android/Sdk/build-tools/35.0.0` | 项目构建版本 |
| Android command-line tools | `/mnt/d/Android/Sdk/cmdline-tools/23.0` | `sdkmanager` 等工具 |
| Java 17 WSL shim | `/mnt/d/DevTools/java17shim` | `JAVA_HOME`；通过包装脚本调用 Windows 的 `D:\bin\java.exe`，不是独立 Linux JDK |
| CMake | `/mnt/d/DevTools/cmake-3.31.10/cmake/data` | CMake 3.31.10；该目录为 `CMAKE_HOME`，`CMAKE_ROOT` 指向其 `share/cmake-3.31` |
| Rust/Cargo 可执行文件 | `/home/jouissance/.cargo/bin` | PATH 中包含该目录 |
| 环境变量配置 | `/home/jouissance/.config/phiraiad/env.sh` | PhiraiAd Android/Rust 工具路径配置 |

环境文件由 shell 启动配置加载。需要查看当前实际生效值时，在 UbuntuD 中运行 `source /home/jouissance/.config/phiraiad/env.sh` 后检查 `env`、`command -v rustc`、`rustup show` 等。


## UbuntuD 系统构建工具

以下工具来自 UbuntuD 系统 PATH（不在 D 盘 SDK 目录内）：

| 工具 | 命令位置 / 版本 | 用途 |
|---|---|---|
| GCC / G++ | `/usr/bin/gcc`、`/usr/bin/g++`；13.3.0 | C/C++ 主机端工具 |
| Python | `/usr/bin/python3`；3.12.3 | 构建辅助脚本 |
| Git | `/usr/bin/git`；2.43.0 | 源码版本管理 |
| pkg-config | `/usr/bin/pkg-config`；1.8.1 | 原生库依赖发现 |
| Make、tar、unzip | UbuntuD 系统命令 | 构建与归档处理 |
| Android Clang | `$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/` | NDK 自带的 Android C/C++ 交叉编译器 |
| Rust 组件 | `/home/jouissance/.cargo/bin` 对应 Rustup 安装 | `clippy`、`rustfmt`、`rust-analyzer` 均已安装 |
## 路径使用提示

- 在 Windows 上，仓库路径是 `D:\phiraiad\PhiraiAd`；在 UbuntuD 中是 `/mnt/d/phiraiad/PhiraiAd`。
- Windows SDK `C:\Users\Lenovo\AppData\Local\Android\Sdk` 和项目 WSL SDK `/mnt/d/Android/Sdk` 不要混为一个 SDK。
- WSL 使用 D 盘工具时，使用 `/mnt/d/...` 路径；Linux 用户目录下的 Rust 安装为 `/home/jouissance/.cargo` 和 `/home/jouissance/.rustup`。
- 本文记录的是工具位置，不代表已验证整个仓库的所有目标平台都能完整构建。构建仍须按对应脚本检查依赖和参数。
