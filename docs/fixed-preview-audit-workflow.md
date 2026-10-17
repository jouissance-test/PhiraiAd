# Fixed Preview 自动审计工作流

本工具用真实生产解析、动画、判定线和 Note 绘制代码产生数据，不用多模态识别视频，也不靠重新实现一份运动公式来证明求解器正确。它用于发现和回归问题，**运行完成不等于没有 Bug**。

## 一、输入与隐私边界

当前本地语料位于 `D:\phiraiad\chart-corpus`，包含用户从手机导出的 5 个完整谱包：Waking Shadows、Dear Farthest You、Submerged City、εxceζ、VVelcome!!。原谱包只读，不上传仓库。每轮先校验 ZIP CRC，并记录整个谱包 SHA-256；因此不能把不同版本的同名谱偷偷当作同一输入。

构建和运行均可在电脑完成，不需要连接手机，不修改手机应用或设置，不播放音乐，不创建可见游戏窗口。

## 二、生产数据出口与独立观察

数据路径：

`GameScene::load_chart → 生产 parser/纹理加载 → Chart::update → JudgeLine::render → Note::render → draw_tex_pts → NoteBuffer::push/geometry`

`prpr` 的非默认 `render-audit` feature 才启用观测 hook。默认游戏不启用此功能。观测出口读取实际生成的四顶点、UV、顶点 alpha 与 Note 身份，结合真实 camera projection 得到最终 NDC。Note 在渲染前被隐藏或裁掉，便不会出现可见绘制记录；求解端另有逐 Note flow、搜索端点状态和回退原因，因此可以区分“求解失败”和“求解成功但实际表现不达标”。当前回退原因仍有粗粒度的 `no_geometry_at_minimum` 等分类，并非所有早退都已细分为完整事件日志。

可见性分析独立对实际提交四边形做 polygon clipping，不调用 Fixed Preview 求解器的 `note_quad_at` / `quad_visible` 来自证。每次引擎启动会先用真实 PGR 解析器和常速测试谱校准坐标、flow 倍率、X 镜像及时间门。校准缺失或失败会阻止可信报告。

批量扫描保留生产 geometry 生成/提交，但采集后丢弃排队的 GPU 光栅绘制，避免每个时间样本都画整幅屏幕。已用 60–64 秒的全部 modes JSON 比较证明：跳过光栅以及启用优化构建，均不改变采集结果。校准仍实际提交到 OpenGL。

## 三、构建

本机 UbuntuD 有 WSLg 隐藏 OpenGL context；脚本只在工具自身的 ignored `host-support/lib` 目录链接已有 ALSA/Wayland runtime，不安装系统包，不下载依赖，不修改系统链接设置。

```sh
source /home/jouissance/.config/phiraiad/env.sh
cd /mnt/d/phiraiad/PhiraiAd
bash tools/preview-audit/host-support/build-host.sh
```

默认生成 `target/debug/phira-preview-audit`，本地 dev 构建 opt-level=2。若配置 `CARGO_TARGET_DIR`，二进制在对应目录。这个优化参数仅作用于该次构建，不修改全局 Cargo 设置。首次构建耗时与缓存有关；后续扫描不重新编译。

## 四、先跑已知失败，再跑整批

### 4.1 精确复现 62 秒问题

```sh
target/debug/phira-preview-audit \
  --chart /mnt/d/phiraiad/chart-corpus/waking-shadows.zip \
  --preview 0.45 --speed 1 --start 60 --end 64 --hz 120 \
  --out /mnt/d/phiraiad/audit-runs/NEW-RUN/waking-62.raw.json

python3 scripts/preview_audit.py check \
  /mnt/d/phiraiad/audit-runs/NEW-RUN/waking-62.raw.json \
  --cases tests/preview-audit/waking-62-regression.json
```

`NEW-RUN` 每轮使用新目录；保留原始证据，不覆盖旧报告。第二条命令的退出码：

- `0`：指定行为回归已达标；
- `1`：已知行为问题仍存在，必须继续诊断；
- `2`：输入、校准、配置或覆盖不足，**不能当作通过**。

这个用例最初确实检出了实现缺陷：line 4 的 Hold（当前解析索引 0）及 Click（索引 27）均在 `62.2461288356` 秒判定，旧实现目标 `450ms`、实际只有约 `96.13ms`，检查命令返回 `1`。最终逐 Note 计划实现让两者选择 `GateOnly`，生产轨迹测得 `446.13ms`，误差约 `-3.87ms`，检查命令返回 `0`。用例仍按谱包哈希、line、判定时间与 Note 类型绑定，并要求 `constant` 或 `gate_only` 支持计划，不能靠改索引、放宽目标或回退 `original` 获得假绿。

### 4.2 三档预判、5 张谱和中途修改

Windows PowerShell 可直接调用标准 Python 外层工具（会自动通过 UbuntuD 启动 Linux 引擎）：

```powershell
python scripts/preview_audit.py run `
  --corpus D:\phiraiad\chart-corpus `
  --out D:\phiraiad\audit-runs\NEW-RUN `
  --engine D:\phiraiad\PhiraiAd\target\debug\phira-preview-audit `
  --previews 0.2 0.45 0.75 `
  --scenario tests/preview-audit/pause_changes.json
```

Linux 下同样使用该 Python 脚本，只需传 Linux 绝对路径。默认全部谱面三档各一遍，另外对 `waking-shadows` 加一遍设置变化场景，共 16 个任务，每个任务都跑关闭／开启两种模式。可以用 `--only waking-shadows`、`--start 60 --end 64` 快速局部复查；`--hz` 默认按现实秒采样 120 次，谱面步长为 `playback_rate / hz`。

场景文件在 61.5、62.5、63.5 秒分别暂停并切换 `(播放速度, 预判秒数)`：`(0.8, 0.2) → (1.2, 0.75) → (1.0, 0.45)`。应用事件的时刻会量化到下一个采样点并记入报告。

**暂停测试分两层，不能混淆：**

- 引擎场景重建生产 render cache 并观察后续真实 renderer。每段都有实际配置对应的 solver 快照。暂停秒数进入统计 wall-clock，但不会推进 chart 时间；跨设置事件的几何跳变单列，不混入连续运动的峰值速度。
- `tests/chart-play/tests/fixed_preview_hot_reload.rs` 和 `pause_resume.rs` 另外提取并执行生产 `FixedPreviewKey`、`sync_fixed_preview`、暂停／恢复方法与 `TimeManager`，用 recording Chart/audio adapter 检查暂停冻结、改速／改预判失效重建、禁用清缓存、恢复速度。它们不是完整手机 UI E2E，也不验证最终声画同步。

## 五、报告与判读

每轮目录包含：

- `corpus.json`：本轮谱包身份；
- `*.raw.json`：原始生产绘制聚合、校准、资源/引擎哈希、solver 与分段快照；
- `*.findings.json` / `.csv`：按严重度排序的 Note 候选及指标；
- `*.job.json` / `.log`：单任务耗时、状态与过程；
- `run.json` / `README.md`：批量执行总览。

需要逐帧深入时直接调用引擎，加 `--frames PATH.jsonl`。默认不保存整谱每一帧的全量 quads，避免过大数据；原始聚合保留首次/末次可见、可见 spans、最大位移及峰值速度对应的两帧四边形。

求解报告中的 `constant` 与 `gate_only` 都是受支持计划：前者采用通过完整区间验证的常量 flow，后者保留原 flow、只从目标时刻开放显示。只有 `original` 才是安全回退，审计器会将它记为 `solver_fallback`，不再对它强求统一预判时长，而是逐项比较 baseline/fixed 的可见帧、spans、首次/末次四边形和运动峰值；任何差异记为 `fallback_visual_mismatch`。提交数量这类纯裁剪优化计数不参与等价判定。已知用例还可用 `allowed_plans` 要求 Note 必须由受支持计划接管，防止“时间碰巧正确但其实已经回退”被误判为修复。

`solver_fallback`、`preview_timing_mismatch`、`visibility_gaps`、`not_visible_near_hit`、`no_fixed_visibility_baseline_visible` 是可量化现象；`speed_amplification_candidate` 与 `screen_interior_appearance_candidate` 只是优先排查线索。谱面有意显隐、Hold 的几何形状变化都可能产生候选，不可把所有 flag 数量当作已确认 Bug 数量。

预判误差默认容差为 `max(20ms, 2/hz)`。只分析判定前的几何数据，避免未执行判定导致命中后的继续绘制污染指标。扫描边界截断首次出现时标记 censored；零 eligible Note 是覆盖不足而不是通过。不同播放速度下按分段 `chart_delta / speed` 计算 active 可读时长，主动暂停不计入预判时长。设置事件后的过渡窗口不判定统一时长、可见 gaps 或 near-hit 缺口，因为它的轨迹由两个有效计划拼接；过渡窗口以外仍按当前分段计划严格验收。

比较两轮同配置摘要：

```sh
python3 scripts/preview_audit.py compare OLD.findings.json NEW.findings.json
```

按现象、solver 状态和 Note 类型聚类，自动保留每组 3 个代表案例（统计会区分配置重复次数与去重 Note 数）：

```sh
python3 scripts/preview_audit.py triage /PATH/TO/RUN --out /PATH/TO/RUN/triage.json
```

同时生成 `triage.md`，这些是候选现象组，不是已证明的根因组。

只允许谱包哈希、viewport、资源身份、采样区间、频率和场景等一致的输入。输出新增、消失和仍存在的现象。引擎版本可以不同——这正是回归比较的目的。差异只表示候选现象发生变化，仍需结合已知用例和轨迹判断改善／退化。

## 六、固定循环与验收边界

1. 新谱包放到本地语料目录，建立本轮 CRC/hash 清单。
2. 先运行精确失败用例，确认检测器能抓住用户原始症状。
3. 跑三档与设置变化场景，将异常按回退原因、几何模式、显隐和时间聚类。
4. 保留完整谱包，再缩小代表例子；不能只截时间片而丢掉此前速度积分、BPM、父线状态。
5. 有证据后修改运动／求解模型；重新执行同一失败用例和整批矩阵，比较新增／消失的现象。
6. 只有已知行为检查达标且无不可接受退化，再做 Android 构建和少量真机体验核验。

当前观测不包含纹理透明像素、GPU shader/postprocessing、framebuffer 遮挡、noise-area 成像、HUD/粒子；PracticeView/post-view 保持默认；Hold 仅覆盖判定前阶段，不覆盖按住后的持续段。默认资源包可能不同于用户手机自定义皮肤，报告记录所用资源身份。离线几何扫描与主机暂停单测都不能替代这些未覆盖项的验收。

数据量可达约 GB 级，语料和报告保存在仓库外；该流程不自动删除任何旧文件。

## 七、历史基线执行记录（2026-10-16）

- 5 张谱均已复制到电脑并完成压缩包 CRC 校验；手机之后不再参与开发或运行。
- 批量报告目录：`D:\phiraiad\audit-runs\20261016-corpus-v2`，15 个整谱三档任务加 1 个整谱设置变化任务全部完成，无引擎失败。任务引擎耗时合计 `266.83 s`（不含构建及 Python 报告处理），单任务约 `9–26 s`。
- 完整报告已聚类为 57 组候选现象；这不是 57 个已确认 Bug。`triage.md` 列出每组代表例子，完整 Note 清单在 JSON/CSV。
- 精确回归在整谱报告上实际返回退出码 `1`；同一 60–64 秒窗口以 `240 Hz` 复查，两个已知 Note 的可见时长仍约 `96.13 ms`，排除仅由 `120 Hz` 粗采样造成这一巨大误差。局部复查目录：`D:\phiraiad\audit-runs\20261016-waking-focus-240`。
- 本轮引擎二进制保存为 `D:\phiraiad\artifacts\preview-audit-49a6cf9d`，SHA-256 `49a6cf9d8a549422881993a3dcf26cfba445c7de30471544c142f3664a82970a`，构建源码与工具提交 `e72fff5` 的引擎内容一致。保留固定二进制避免后续构建覆盖正在运行的任务；每份 raw 报告还自记 binary 和资源包身份。
- 本轮只建立观测与回归循环，没有修改运动算法、没有宣称修复 62 秒问题，也没有生成或安装新 Android APK。
- 主代理复跑：39 项配置/几何/策略/UI/暂停与热更新测试、4 项扫描引擎指标测试、14 项 Python 工作流测试全部通过。它们验证工具和既有行为；独立的 62 秒游戏行为检查仍为红色。

## 八、最终计划实现执行记录（2026-10-17）

- 生产代码修订 `2c802ada1a34917a9607b11ad5c0a3ad11e85325`；审计引擎保存为 `D:\phiraiad\artifacts\preview-audit-2c802ada1a34`，SHA-256 为 `917d6f309bc707b2f000b8bfc49c00fc9f7a97e19dfbb698b4528a7879ea4366`。
- 最终完整目录：`D:\phiraiad\audit-runs\20261017-corpus-motion-final`。5 张谱三档加 Waking 动态设置场景共 16 个任务，引擎失败与校准失败均为 0。
- `preview_timing_mismatch`、`visibility_gaps`、`not_visible_near_hit`、`no_fixed_visibility_baseline_visible`、`fallback_visual_mismatch` 五类契约错误在 16 个任务中均为 0。启发式“屏内首次出现”和“速度放大”候选仍单列，未被包装成已确认 Bug 或全通过结论。
- Waking 62 秒两个绑定用例均为 `GateOnly`，目标 `450ms`、实测 `446.13ms`、误差 `-3.87ms`，定点检查退出码为 0。
- 动态场景依次执行 `0.45s/1.0x → 暂停 1.0s → 0.2s/0.8x → 暂停 0.5s → 0.75s/1.2x → 暂停 0.5s → 0.45s/1.0x`。设置切换窗口单列，窗口外没有上述五类契约错误。
- 完整逐谱统计、MotionSpike 回退数量和测量边界见同目录 `FINAL_REPORT.md`。本轮不连接、不唤醒、不操作手机，因此不把主机生产轨迹审计称为新一轮真机视觉验收。
