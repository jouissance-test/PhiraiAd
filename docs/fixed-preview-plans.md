# Fixed Preview Time：逐 Note 视觉计划

本实现只改变渲染，判定、音频、谱面时间、判定窗、成绩与原始速度事件不变。每个 Note 在缓存重建时取得一个计划，渲染时只读取计划和比较时间，不分配内存、不求解。

- `GateOnly { original_flow, start }`：原始 flow 已在目标到判定前的整个采样区间可见，只在目标之前隐藏。
- `Constant { flow, start }`：原始 flow 不满足区间要求，目标点二分候选或随后细化出的较小常量候选通过完整区间验证后才采用；目标前隐藏。
- `Original { original_flow, reason }`：不可靠或不支持时恢复原始 flow 和原始显隐，完全不施加 Fixed Preview 时间门。fake Note 始终使用此计划。

选择顺序为 GateOnly → Constant → Original。Hold 的 head/body/tail 使用同一份计划。现有暂停、改速、改预判、视口变化等缓存失效机制继续负责重建；缓存键额外包含当前有效原始 flow。Original getter 始终读取当前 fallback flow，保留回退原因，避免设置变化与下次缓存同步之间固化旧速度。

先检查原始 flow；若不能使用 GateOnly，则在目标点二分求出最大可见常量候选并验证完整区间。目标点候选失败时，只在最小安全 flow 本身通过区间验证的前提下，以 10 次二分细化出最大的安全较小常量候选；最小值也失败便立即恢复 Original。每个候选以现实秒 240Hz 的最大间隔检查 target、区间内部和 hit 左极限，每个候选最多 1537 次几何查询；超过 1536 步的区间恢复 Original。验证可在第一个失败样本处提前拒绝。这是有明确预算的采样验证，不能证明任意小于约 4.17ms 的事件不会落在采样之间，也不是多段或动态 flow 求解。

Constant 还检查同一区间相邻采样的语义 Note 头锚点运动，按真实时间间隔归一化为 `distance_ndc / (delta_chart / playback_rate)`。候选允许继承谱面判定线本来就有的高速运动，但不能比同一时刻的 Original 轨迹更快到足以构成新尖峰：允许峰值为 `max(24 NDC/现实秒, Original 同时段速度 + 1 NDC/现实秒)`。绝对下限拦截原谱几乎不动时由候选新制造的高速，`+1` 用于吸收采样与浮点噪声。超过限制时返回 `Original(reason=motion_spike)`，恢复原始 flow 和原始显隐；不会因为作者原有的整线高速运动而机械回退。

GateOnly 保留原谱运动，即使原谱有高速或跳变也不使用这个上限。Tap/Flick/Drag 使用中心锚点；Hold 使用 `local(0, base*flow)` 经 Note、line、reflection 和 PracticeView 变换后的语义 head 位置，不受皮肤 head 贴图偏移、compact 模式或 body/tail 长度变化影响。该检查只覆盖 Hold head 的运动；仅 tail 发生的尖峰仍未覆盖。

`prpr/src/note_visual.rs` 由生产 Note 渲染和求解采样共同使用，统一 Note 旋转、scale、control size/pos、incline 与 flow 变换，以及 line 旋转/平移。line.object.scale 只影响 carrier 自身绘制，不能乘到 Note 上。求解端另对 Hold 实际 head/body/tail 三个四边形分别判断，反向且未绘制的 body 不作为两端之间的可见包围盒。

求解器只认证旅行几何是否连续留在 viewport，不把 Note alpha、control alpha、负 line alpha、beat appearance 或 cover 当成“几何不存在”。这些作者显隐规则仍由生产 renderer 独立执行，Fixed Preview 不会绕过或改写它们；审计出口目前能把由 Note/control/部分负 line alpha 区间解释的短显隐或预判差异单列，其他作者显隐仍会保持严格契约失败，等待人工确认，不能被误豁免。仍未共享的是生产绘制提交、纹理尺寸/UV 切片和最终 camera/postprocessing。透明纹理像素、shader、遮挡、noise-area、HUD 和按住 Hold 之后的可读性不在区间几何验证内；实际生产 renderer 的独立审计继续作为回归出口。

`audit_preview_solver` 通过与缓存重建相同的 `note_plan` 返回 `plan`、`flow` 和 `reason`，并记录目标边界、最小／边界／原始候选的几何与运动诊断、作者显隐区间，以及 Constant 的绝对和相对运动阈值。`constant` 的 reason 保持 `solved`，`gate_only` 是受支持计划；只有 `original` 是原样回退。历史 min/max 几何诊断字段保留，不能再单独用 `reason != solved` 认定失败。
