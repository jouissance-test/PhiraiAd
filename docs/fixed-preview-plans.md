# Fixed Preview Time 技术设计

## Why：问题与目标

谱面原始 flow、判定线动画、Note 控制事件和作者显隐共同决定 Note 何时进入视野，因此单纯设置一个全局 flow 不能保证一致的可见预判时间。已观察到的失败包括同一段谱面中部分 Note 明显晚出现，以及候选 flow 将原本在屏幕外的高速位移暴露成突然冲过视野。

本功能的目标是在不改变音频、谱面时间、判定窗口和成绩的前提下，让尽可能多的非 fake Note 从进入可视区域到判定的现实时间接近用户设置值。不能可靠认证的 Note 必须完整恢复原谱表现，而不是带着不完整的时间门继续运行。

## What：用户可见行为

- 功能默认关闭，位于“改版设置 → 改版玩法”，标注为实验性功能。
- 默认值为 `0.450s`，范围 `0.100–0.750s`，步进 `0.001s`。
- 暂停页支持单击微调与长按加速；暂停、变速、改值、视口和相关渲染参数变化都会重建缓存。
- 仅改变 Note 的视觉 flow 和首次显示时间；不改变音频、谱面、判定和成绩。
- 当前不实现动态或多段 flow；无法使用安全常量计划时回退原谱。

## How：逐 Note 计划与求解

本实现只改变渲染，判定、音频、谱面时间、判定窗、成绩与原始速度事件不变。每个 Note 在缓存重建时取得一个计划，渲染时只读取计划和比较时间，不分配内存、不求解。

- `GateOnly { original_flow, start }`：原始 flow 已在目标到判定前的整个采样区间保持旅行几何可见，并且目标时刻不受谱面作者显隐门阻止，只在目标之前隐藏。
- `Constant { flow, start }`：原始 flow 不满足要求，目标点二分候选或随后细化出的较小常量候选通过完整区间旅行几何验证、且候选自己的 flow 在目标时刻通过谱面作者显隐门后才采用；目标前隐藏。
- `Original { original_flow, reason }`：不可靠或不支持时恢复原始 flow 和原始显隐，完全不施加 Fixed Preview 时间门。fake Note 始终使用此计划。

选择顺序为 GateOnly → Constant → Original。Hold 的 head/body/tail 使用同一份计划。现有暂停、改速、改预判、视口变化等缓存失效机制继续负责重建；缓存键额外包含当前有效原始 flow。Original getter 始终读取当前 fallback flow，保留回退原因，避免设置变化与下次缓存同步之间固化旧速度。

先检查原始 flow；若目标时刻被作者显隐门阻止，只禁止 GateOnly，仍继续尝试 Constant。这样不会安装一个“时间门已打开、renderer 仍不画”的假支持计划，同时允许 `shorten_holds + hold_partial_cover` 下由候选 flow 改变有效 Hold 尾高、从而真实通过 cover 的少数 Note 继续统一预判。若没有目标旅行几何边界 bracket，但最小 flow 能通过完整验证，则在最小 flow 到已知失败的原始 flow 之间细化安全候选；这同时覆盖“目标时刻所有 flow 都在屏幕内、但原 flow 稍后离屏”的情况。

若端点没有 bracket 且最小 flow 也失败，计划器以 64 段只探测目标时刻，寻找端点之外的最高 flow 可见岛，再最多验证岛内右缘、中点和左缘三个常量候选。若正常边界候选与最小候选分别因运动尖峰和区间出视野失败，则在两者之间最多验证 8 个常量探针。所有探针仍执行完全相同的完整区间、作者显隐差分和运动保护；探测本身不会把“目标某一点可见”当作支持，也不降低任何验收阈值。找不到安全常量便恢复 Original，仍不做多段或动态 flow。

其他情况在目标点二分求出最大可见常量候选并验证完整区间。目标点候选失败时，只在最小安全 flow 本身通过验证的前提下，以 10 次二分细化出最大的安全较小常量候选；最小值也失败便进入上述有界内部探测或恢复 Original。

每个 Constant 候选以现实秒 240Hz 的最大间隔检查 target、区间内部和 hit 左极限的旅行几何，每个候选最多 1537 个采样；超过 1536 步的区间恢复 Original。作者显隐在 target 时刻是接管前置条件；target 之后若候选和 Original 同时被作者门隐藏，该段不要求旅行几何在 viewport 内，并在重新出现时重开运动连续段，从而保留谱面作者制作的透明度闪烁或隐藏。若候选 flow 新制造了 Original 不存在的隐藏，则拒绝候选；未经双方共同作者门解释的消失仍是严格契约失败。验证可在第一个失败样本处提前拒绝。这是有明确预算的采样验证，不能证明任意小于约 4.17ms 的事件不会落在采样之间。

Constant 还检查同一区间相邻采样的语义 Note 头锚点运动，按真实时间间隔归一化为 `distance_ndc / (delta_chart / playback_rate)`。只有 Original 在相邻两个采样都通过作者显隐门且旅行几何实际位于 viewport 内时，候选才允许继承谱面判定线本来就有的高速运动；此时允许峰值为 `max(24 NDC/现实秒, Original 同时段速度 + 1 NDC/现实秒)`。若 Original 当时透明或在屏幕外，则候选只能使用 24 NDC/现实秒的绝对上限，不能把 Original 原本不可见的高速跳变暴露出来。`+1` 用于吸收采样与浮点噪声。超过限制时返回 `Original(reason=motion_spike)`，恢复原始 flow 和原始显隐。

GateOnly 保留原谱运动，即使原谱有高速或跳变也不使用这个上限。Tap/Flick/Drag 使用中心锚点；Hold 使用 `local(0, base*flow)` 经 Note、line、reflection 和 PracticeView 变换后的语义 head 位置，不受皮肤 head 贴图偏移、compact 模式或 body/tail 长度变化影响。该检查只覆盖 Hold head 的运动；仅 tail 发生的尖峰仍未覆盖。

`prpr/src/note_visual.rs` 由生产 Note 渲染和求解采样共同使用，统一 Note 旋转、scale、control size/pos、incline 与 flow 变换，以及 line 旋转/平移。line.object.scale 只影响 carrier 自身绘制，不能乘到 Note 上。求解端另对 Hold 实际 head/body/tail 三个四边形分别判断，反向且未绘制的 body 不作为两端之间的可见包围盒。

求解器只认证旅行几何是否连续留在 viewport，不把作者显隐当成“几何不存在”。计划器另以与 renderer 相同的谱面数据检查目标时刻的 Note alpha、control alpha、负 line alpha 模式、beat `appear_before` 和 cover；这些规则不会被 Fixed Preview 绕过或改写。若 GateOnly/Constant 都无法在目标时刻通过，返回 `Original(reason=authored_visibility_at_target)`，完整恢复原始 flow 和原始显隐。混合候选失败只有在边界和最小候选都受作者门阻止时才使用此原因，避免掩盖几何区间或运动尖峰失败。

“作者显隐”有意不包括玩家 FADE_IN/FADE_OUT Mod、场景级 `res.alpha` 淡入、纹理透明像素、shader 或其他遮挡；这些瞬时或后处理状态不应固化进逐 Note 缓存。仍未共享的是生产绘制提交、纹理尺寸/UV 切片和最终 camera/postprocessing。noise-area、HUD 和按住 Hold 之后的可读性也不在区间几何验证内，因此设备体验仍是合并前的重要验证出口。

## Compatibility：兼容性与回滚

- 默认关闭时保留原来的 Note flow、作者显隐和渲染时序。
- 旧配置中的非有限值或超范围值会在加载时归一化，不需要迁移谱面、资源包或用户数据。
- fake Note 始终使用 `Original`；不能安全处理的普通 Note 也恢复原 flow 和原显隐。
- 缓存键包含启用状态、目标时间、播放倍率、视口、Note 宽度、翻转、练习视图、Hold 选项和当前原始 flow，避免跨设置复用旧计划。
- 实现不增加新的平台专用运行时依赖；功能可关闭，整个变更也可通过回滚恢复。

## Testing：已验证范围

- 73 项 Rust 测试覆盖配置归一化、三类视觉计划、Note/Hold 几何、作者显隐、cover、运动尖峰、暂停热更新、播放倍率、长按调节和设置分组。
- Android `aarch64-linux-android` 目标上的 `prpr` 编译检查通过；最终 ARM64 APK 已完成构建、签名与 16 KiB 对齐检查。
- 私有 15 谱四档预判矩阵及动态设置场景共 61 次运行，无引擎或视觉契约失败；总体支持覆盖率为 `95.874%`，其余 Note 安全回退 `Original`。
- 最终 APK 已随机试玩几张谱，存在少量可接受回退，未报告明显的新异常。

谱面语料和批量 renderer 审计工具不属于本 PR：语料不提交，审计基础设施保留在开发分支。仓库内的合成测试覆盖已发现的主要回归形状。

## Limitations：已知限制与讨论点

- `Constant` 使用 240Hz 现实时间采样和有限预算验证，不能数学证明采样间隔内绝无更短事件。
- 运动保护检查语义 Note 头锚点；Hold 仅 tail 独立发生的尖峰尚未覆盖。
- 不验证纹理透明像素、shader/postprocessing、HUD/粒子遮挡或按住 Hold 后的完整可读性。
- 部分谱面在较长预判时间下会有更多 Note 回退；当前选择安全回退，而不是放宽连续可见或运动约束。
- 动态或多段 flow 可能提升极端谱面的覆盖率，但会显著增加运行时状态和视觉连续性风险，本 PR 有意不实现。维护者可讨论是否值得作为后续功能。
