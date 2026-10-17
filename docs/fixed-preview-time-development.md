# Fixed Preview Time 开发说明

本文记录 `D:\phiraiad\功能实现说明.md` 对应的源码核查、首版架构和验收边界。源码基线为 `v1.5.2`（`a60b49a`）。

## 1. 源码核查结论

### 1.1 现有 flow 已经是渲染专用量

- `prpr/src/config.rs` 的 `Config::global_note_flow_speed()` 对持久化值做有限值检查和裁剪；`effective_note_flow_speed()` 在其上叠加练习模式倍率。
- `prpr/src/scene/game.rs` 的 `sync_note_flow_speed()` 每帧把当前模式的有效值写入 `Resource::note_flow_speed`。
- `prpr/src/core/note.rs` 在普通 Note 的最终局部变换、Hold 的 head/body/tail、缩短 Hold 的绘制尺度，以及回放录制中心采样中读取这个渲染倍率。
- 判定时钟、音频时钟和判定窗口不读取该值。因此需求中“只改变视觉、不改变 chart/audio/judgement”的分层在当前架构中成立。

需求说明中“当前仅练习模式使用 `note_flow_speed`”已被现有源码演进取代：现在它是全局基础倍率，练习模式可再叠加临时倍率。这不影响逐 Note 扩展，但实现不能再把它视为 Exercise 私有设置。

### 1.2 Hold 必须在同一个调用层取得 flow

`prpr/src/core/note.rs` 中：

- 普通 Note 的中心位移由 `Note::now_transform()` 应用 flow；
- Hold 的 head、body、tail 共用 `bottom/top`，两者都由同一 flow 缩放；
- Hold 已开始判定后，body 底端固定在判定线，尾端仍使用同一 flow；
- 回放中心采样和缩短 Hold 的绘制尺度也依赖 flow。

因此首版不把倍率写入谱面或 speed event，而是给上述渲染入口显式传入同一个“当前 Note 的有效 flow”。这样普通 Note 与 Hold 的所有几何保持连续。

### 1.3 viewport 与 RPE 动画可以无副作用采样

- `Anim::value_at(time)`（`prpr/src/core/anim.rs`）允许不移动播放 cursor 地读取任意 chart time。
- 判定线位置、旋转、缩放、父子线变换在 `prpr/src/core/line.rs` / `object.rs` 中可重建。
- Note 的 yOffset、旋转、缩放以及 RPE control 的 `y/pos/size/alpha` 都可用同一套动画值采样。
- `Resource::chart_view_points()` 和 `PracticeView` 提供当前 viewport 的平移/缩放语义；求解器还需要补上 camera aspect，得到实际 NDC 的 `[-1, 1]` 可见矩形。

因此不需要 clone 整张谱或临时改写运行中的动画 cursor。

### 1.4 现有 early-break 与逐 Note flow 不兼容

`prpr/src/core/line.rs` 的 plain-note 分组使用单个 `Resource::note_flow_speed` 计算高度上限，并在后续 Note 看似都在 viewport 外时 `break`。逐 Note flow 会破坏“同组后续 Note 共用相同可见上限”的前提。

首版启用 Fixed Preview Time 时关闭这两个 aggressive break；仍保留 `draw_tex_pts` 的真实逐 Note clipping。关闭功能时原优化完全不变。

### 1.5 模式、时钟和范围

- `GameMode::{Normal, NoRetry, Exercise, TweakOffset}` 都使用同一个 `GameScene`、`Chart` 和 `JudgeLine::render()`；挑战玩法以 `GameMode::Normal + Config::challenge_mode` 进入同一路径。
- 这些模式的 `TimeManager::speed` 和 `MusicParams::playback_rate` 都来自当前会话的 `Config::speed`；Nightcore 在场景创建时先合入该值。因此 `target_preview_chart = target_preview_real * Config::speed` 是当前源码中实际生效的 chart-clock 倍率。
- `View` 是回放查看；`EditChartPlay` 是谱面编辑预览，首版明确排除。
- 当前联机房间从 `phira/src/mp/panel.rs` 以 `GameMode::NoRetry` 启动同一个 `GameScene`，仓库里没有第二套独立 Note renderer。首版不新增或承诺独立联机渲染路径；共用 `GameScene` 的现有 NoRetry 路径会自然使用同一渲染规则。

## 2. 首版模块与 seam

逐 Note 缓存放在 `JudgeLineCache`，对调用方只提供两个行为：

1. 根据当前 Fixed Preview key 重建全谱缓存；
2. 渲染某 Note 时取得其有效 flow，失败项返回原全局 flow。

求解、动画采样、几何重建、viewport 相交、上下界和诊断统计全部藏在该 seam 后。`Note` 本身不新增谱面字段，解析器和二进制谱格式不变。

`GameScene` 保存一个轻量 cache key，包含：

- 开启状态与目标现实秒数；
- 当前实际 playback rate；
- viewport 尺寸、aspect、练习视角；
- 会影响 Note 几何的 note width / flip / Hold 显示选项。

key 改变时重建；普通 render/update 帧只比较 key 并读取缓存，不运行迭代求解。

## 3. 求解算法

对每个 Note：

1. `target_chart = configured_real_seconds * playback_rate`；
2. `target_time = hit_time - target_chart`；若早于 chart start，则按需求回退原渲染；
3. 在 `target_time` 无副作用采样判定线、父线、Note、control event 和 Hold 尾端；
4. 重建与实际渲染一致的普通 Note 四边形，或 Hold head/body/tail 的连续外包四边形；
5. 应用 chart reflection、判定线/Note transform、练习 viewport 和 aspect，得到 NDC；
6. 用“四边形与 `[-1, 1]` 矩形相交”定义可见，而不只用中心点；
7. 在 `0.1..20.0` 内先验证存在“低 flow 可见、高 flow 不可见”的 bracket，再做固定 16 次二分，返回边界内侧的有限正数。

`0.1..20.0` 直接复用当前项目 `NOTE_FLOW_SPEED_MIN/MAX`，这两个界限已经由练习 UI、倒数锁定和渲染路径使用。16 次二分在该区间提供远高于像素级需求的精度，且只在缓存重建时执行。

以下情况回退原全局 flow，并累计一次诊断计数：

- 目标时刻早于 chart start；
- 输入或变换产生非有限值；
- 在安全区间内无法形成可见/不可见 bracket；
- 特殊动画令可见性不满足本首版的单调 bracket 假设。

正常渲染还保留单向 guard：`current_time < hit_time - target_chart` 时不绘制。它只阻止求解/特殊谱导致的过早出现，不能让过晚的 Note 提前出现。

## 4. 配置与 UI

`Config` 新增两个向后兼容的 serde-default 字段：

- `fixed_preview_time_enabled = false`；
- `fixed_preview_time = 0.45`，初始化和读取时裁剪到 `0.1..0.75`（2026-10-16 按用户要求收紧上限；旧版保存的超限值也会裁剪）。

全局入口位于“改版设置 → 改版玩法”，标为“统一预判时间（实验性功能）”，不再放在“谱面”页。提供开关和 `0.001 s` 步进滑条，所有入口固定显示三位小数。所有首版支持模式的暂停菜单提供同一组控件，标为“统一预判（实验性）”；暂停修改直接更新当前 `Resource::config`、持久化到应用全局配置，并在下一绘制帧因 key 变化而重建缓存，不 reload chart/audio/texture/scene。

开启时保留手动 Note Flow Speed 的已保存值，但全局 Mods 面板与练习暂停菜单中的手动 flow 控件显示为禁用且不接受修改；关闭后恢复原值和原有模式逻辑。

## 5. 实际修改范围

- `prpr/src/config.rs`：配置、范围常量、归一化和序列化测试。
- `prpr/src/fixed_preview.rs`：持久化回调、纯几何相交和有界求解。
- `prpr/src/core/{line,note,resource}.rs`：缓存 seam、逐 Note flow、Hold、guard、early-break。
- `prpr/src/{judge,judgement_range}.rs`：Bad 残影和判定区间调试绘制跟随逐 Note flow。
- `prpr/src/scene/game.rs`：支持模式、cache key、热更新与暂停 UI。
- `phira/src/lib.rs`：把暂停修改写回全局配置的回调。
- `phira/src/page/settings.rs`：全局设置入口。
- `phira/src/scene/song.rs`：Fixed Preview 开启时禁用手动 Note Flow Speed。

## 6. 首版已知边界

- Fade mod、谱面自定义 alpha/appear-before 等可以有意让 Note 在几何进入 viewport 后仍不可见；Fixed Preview 不覆盖这些视觉特效，只保证 flow 控制的旅行几何与 safety guard。
- fake / 表演 Note 的名义 hit time 不等于表演时段；它们不参与固定预判求解或 safety guard，继续使用原全局 flow 和原谱出现规则。
- 无 bracket 的极端父子线运动、反向/零速或特殊 Hold 几何安全回退，不尝试重写谱面运动系统。
- 回放查看和谱面编辑明确不启用。仓库当前联机共用 NoRetry 实时场景，没有独立 renderer 可扩展或单独验收。

## 7. 首版历史验证记录

- `cargo check -p prpr --target aarch64-linux-android`：通过。
- `cargo check -p phira --lib --target aarch64-linux-android`：通过，覆盖应用设置页、暂停持久化桥接和完整 Android 库。
- `cargo check -p prpr --tests --target aarch64-linux-android`：通过，覆盖配置序列化测试以及 viewport 四边形相交/有界求解测试的编译。
- 用户首轮试用反馈后，默认目标改为 `0.45 s`，所有入口使用 `0.001 s` 步进并显示三位小数；暂停菜单 checkbox 绘制明确勾号。
- fake / 表演 Note 不进入逐 Note flow 求解，不使用固定预判 flow，也不经过按名义 hit time 的提前隐藏规则。
- 练习暂停菜单的 Fixed Preview 控件采用独立缩放布局，避开时间轴矩形与居中的时间文字；针对 16:9 和 20:9 viewport 的布局断言通过。
- `tests/chart-play` 中 Fixed Preview 配置、fake Note 策略与练习 UI 的 14 个定向测试通过。
- ARM64 release + video 构建通过，并产出本地测试签名的 `D:\phiraiad\artifacts\PhiraiAd-fixed-preview-test2-arm64.apk`；打包器已验证 v1/v2/v3 签名、16 KiB 对齐、ZIP CRC、manifest 和包内原生库一致性。
- Linux host 目标会在进入本次代码前因环境缺少 `wayland-client.pc` 失败；Android 测试二进制不能直接在 WSL host 执行，因此本轮没有声称完成真机视觉验收。

现有 Android Studio AVD `Codex_Pixel_8_API_36_1` 是 x86_64，而当前 Android 发布链固定生成 ARM64 APK，因此该 AVD 不能安装上述测试包。真机视觉验收仍需 ARM64 设备；若要使用该 AVD，需要另行补齐 x86_64 原生库与 APK 打包路径。

编译使用仓库文档指定的 UbuntuD、nightly toolchain 和 NDK r27d，并按 `scripts/build_android_release.sh` 设置 API 21 的 Android C/C++ linker。构建只产生既有 warning，没有本功能新增的编译 warning。

## 8. 2026-10-16 透明承载线修复

### 复现与根因

用户在真机 `Waking Shadows (feat. Eili)` 的约 50–60 秒反馈：目标预判 `0.8 s` 明显、`0.7 s` 也可见，一部分 Note 减速而另一些仍接近原速。本轮从应用导出实际谱包到 `D:\phiraiad\artifacts\waking-shadows.zip`，只读分析，不修改用户谱面。

`note_quad_at()` 原先在目标采样时刻的 **判定线 alpha ≤ 0** 时拒绝构造 Note 几何，导致求解失败并回退原全局 flow。然而 `JudgeLine::render()` 的 alpha 为 0 只隐藏承载线本身；线上 Note 仍按自己的 alpha/control alpha 正常绘制。实际谱面这段大量使用这样的透明承载线，因而错误地混入了原速 Note。

修复只移除求解器对承载线 alpha 的拒绝，保留 Note 自身和 control alpha 的检查。负 alpha 的 RPE/PEC 显隐扩展继续由实际 renderer 决定；求解成功不会绕过 renderer 的隐藏逻辑。未改变音频、判定时钟、原谱事件、flow 安全区间或 fake Note 策略。

### 回归与边界

- 新主机测试从生产 `fixed_preview.rs` 提取几何采样和求解函数，运行真实矩阵变换、动画采样、viewport 相交与有界求解，不复制求解器。
- 修复前：3 项新测试中 2 项失败（alpha=0 时返回 `None`，同几何不透明时可求解）；修复后 3 项全部通过。另有基于实谱 Note 属性、冻结场景以隔离 alpha 的用例；它不是完整动态谱面回放。
- 暂停滑条原先固定以 `1` 为中点；上限收紧到 `0.75` 后，中点必须随范围调整，否则右半段映射失真。现仅在范围严格包含 `1` 时保留原中点，其余范围采用算术中点；增加整条短范围滑条单调性及端点/触点位置回归，修复前失败、修复后通过。
- 配置、几何、fake policy、练习 UI、设置分组共 26 项定向测试通过。
- 扩展执行整个 `tests/chart-play` 测试集时，旧 `best_board_ui`/库内 practice-view 适配器缺少类型、方法和数学函数而无法编译；这些非本次功能的测试未宣称通过。
- 判定线在目标时刻仍在屏外、原谱有跳变/零速、无可见性 bracket 或有特殊显隐时，仍可能安全回退；固定预判不等于所有 Note 的全程屏幕像素速度相同。因此保留实验性标记及 `0.75 s` 上限。

定向回归命令：

```sh
cargo test --offline --manifest-path tests/chart-play/Cargo.toml \
  --test fixed_preview_geometry --test fixed_preview_config \
  --test fixed_preview_policy --test practice_ui --test settings_ui
```

### 构建与实机材料

- UbuntuD `nightly-2026-09-14` / NDK r27d：`cargo build --locked -p phira --lib --target aarch64-linux-android --release --features video` 通过。包含透明线修复的中间包及最终滑条修正均分别完成 release 构建，只有既有 warning。
- 同一 ARM64 真机、播放倍率 `1.00`、预判 `0.700 s`、练习起点 `50.55 s`，保存修复前/后的 15 秒录屏：`D:\phiraiad\artifacts\phiraiad-baseline-0700.mp4` 和 `phiraiad-fixed-0700.mp4`。前者已含上限/页面修改但沿用旧求解器，后者含透明线修复；两段没有做逐帧时钟同步，不据此声称每个 Note 的预判误差都已量化消失。
- 设置迁移与实验性标注的真机截图：`D:\phiraiad\artifacts\phiraiad-replica.png`。应用覆盖安装保留原有谱面与账号，不卸载或清数据。
- 最终包：`D:\phiraiad\artifacts\PhiraiAd-fixed-preview-test3-arm64.apk`，版本 `1.6.0-fixed-preview-test3` / `12004`，SHA-256 `ce005ea5c2d53b112972fb76936faac4426967a80be2366be7558ce3798a1c41`。本地测试证书的 v1/v2/v3 签名、16 KiB 对齐、ZIP CRC、manifest、原生库匹配检查均通过；随后 ADB 覆盖安装成功，并以 `dumpsys package` 核对安装版本。
- 最终包真机确认：暂停页实验性文字无重叠；滑条最右端为 `0.750`，减一次为 `0.749`、加回后继续加仍为 `0.750`。最终 `0.750 s` 运行录屏保存为 `D:\phiraiad\artifacts\phiraiad-final-0750.mp4`，控件截图为 `phiraiad-final-pause.png` / `phiraiad-final-step.png`。完成后退出播放、返回 Android 桌面，恢复测试前的充电常亮配置值 `15`，发送 `KEYCODE_SLEEP`，设备进入息屏/Dozing 状态（不更改用户的息屏显示偏好）。

## 9. 2026-10-17 最终逐 Note 计划与自动审计闭环

首版“求出单个 flow，失败也保留统一时间门”的组合会在反向运动、显隐窗口和高速父线运动中制造两类问题：flow 虽在目标点可见，却不能保证目标到击打全程可见；求解失败后若仍保留时间门，也不再等价于关闭功能的原谱。最终实现把每个非 fake Note 明确分成三个视觉计划：

- `GateOnly`：原始 flow 在目标到击打的完整区间已经可见，只增加目标时间门；
- `Constant`：候选常量 flow 通过同一生产几何采样器的完整区间验证和运动保护后才接管；
- `Original`：无法认证时同时恢复原始 flow 与原始显隐，不保留时间门。

生产渲染和求解采样共用 `note_visual.rs` 的 Note 变换语义；Hold 的 head/body/tail 共用一份计划。区间验证按现实时间 240Hz 采样，最多 1536 步，不实现阶段 3 的多段动态 flow 搜索。后续覆盖率优化允许 Constant 继承作者高速判定线运动，但要求候选速度不超过同一时段 Original 峰值加 `1 NDC/现实秒`，并保留 `24 NDC/现实秒` 的绝对保护下限。完整语义和边界见 `docs/fixed-preview-plans.md`。

暂停页 `fixed_preview_time` 的减号/加号现在保留短按 `0.001s` 单步，并在按住约 `350ms` 后按真实单调时钟重复，速率约从 7 步/秒加速到 22 步/秒；松开不再额外多走一步。长按只作用于统一预判时间，其他练习滑条保持原来的点击行为。全局入口仍位于“改版设置 → 改版玩法”，暂停页与设置页继续标明“实验性功能”，持久化范围为 `0.100..0.750s`。

最终五谱矩阵以真实生产 renderer 捕获而不是视频识别：5 张谱的 `0.2/0.45/0.75s` 三档，加一组暂停、变速、途中修改预判，共 16 个任务且无引擎失败。Waking `62.246s` 的 Hold/Click 在 `0.45s` 下实测 `446.13ms`，误差 `-3.87ms`；五类严重契约错误在全部任务中均为 0。详细数量、候选信号和未覆盖边界见 `D:\phiraiad\audit-runs\20261017-corpus-motion-final\FINAL_REPORT.md`。本轮没有连接或操作手机，结果不冒充真机 framebuffer 或肉眼体验验收。

## 10. 2026-10-17 回退比例优化

自动审计证明首版安全计划并非只是“谨慎”，还存在模型过粗造成的可避免回退：目标时刻最大的可见 flow 可能在中途离屏，而更小的常量 flow 可以全程安全；同时，作者本来就有的判定线高速会触发绝对运动阈值，alpha/cover 显隐也曾被混入几何缺失。

本轮保持求解与渲染共享同一 Note 变换语义，并进一步把纯旅行几何和 renderer 作者显隐拆开：Fixed Preview 只认证几何连续性，实际 alpha、appearance 与 cover 仍由 renderer 执行。目标边界候选失败后，若 `NOTE_FLOW_SPEED_MIN` 候选全程安全，就以 10 步二分细化出最大的安全较小常量 flow。Constant 的运动保护同时采样 Original，只拒绝相对原轨迹新增加的尖峰。审计输出保存三个代表候选、65 点无 bracket 探针、几何/可见运动峰值和已支持的作者 alpha 显隐区间，方便下一轮按原因聚类；未建模的作者显隐不会自动获得豁免。

五谱三档加动态设置的候选矩阵共 16 个任务，引擎失败与五类契约失败均为 0。与首版相比，Excez 三档覆盖率由 `86.395/77.551/67.075%` 提升到 `98.844/97.823/96.054%`，Waking 由 `94.307/89.311/80.423%` 提升到 `99.842/99.842/98.492%`，Welcome 由 `88.682/84.437/69.003%` 提升到 `100/99.228/95.691%`，Submerged 保持在 `99.974/99.923/99.207%`。Dear 提升到 `93.607/88.128/82.572%`，后两档仍未达到明确的 90% gate。

对 Dear 剩余回退逐 Note 探针显示，大部分 Note 在完整区间对所有允许的常量 flow 都会离开 viewport；把实验 flow 下限从 `0.1` 降到 `0.0001` 也只救回 `0.45s` 的 1 个和 `0.75s` 的 2 个回退，不能改变结论。继续覆盖它们需要阶段 3 多段/动态 flow 或改变作者判定线运动，已按用户允许的范围暂缓；正式实现保留安全 Original 回退，不以极端压缩换取表面覆盖率。
