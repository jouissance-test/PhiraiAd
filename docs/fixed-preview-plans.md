# Fixed Preview Time：逐 Note 视觉计划

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
