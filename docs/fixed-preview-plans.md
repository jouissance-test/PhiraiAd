# Fixed Preview Time：逐 Note 视觉计划

本实现只改变渲染，判定、音频、谱面时间、判定窗、成绩与原始速度事件不变。每个 Note 在缓存重建时取得一个计划，渲染时只读取计划和比较时间，不分配内存、不求解。

- `GateOnly { original_flow, start }`：原始 flow 已在目标到判定前的整个采样区间可见，只在目标之前隐藏。
- `Constant { flow, start }`：原始 flow 不满足区间要求，目标点二分候选通过完整区间验证后才采用；目标前隐藏。
- `Original { original_flow, reason }`：不可靠或不支持时恢复原始 flow 和原始显隐，完全不施加 Fixed Preview 时间门。fake Note 始终使用此计划。

选择顺序为 GateOnly → Constant → Original。Hold 的 head/body/tail 使用同一份计划。现有暂停、改速、改预判、视口变化等缓存失效机制继续负责重建。

验证最多检查两个候选。每个候选以现实秒 240Hz 的最大间隔检查 target、区间内部和 hit 左极限，每个候选最多 1537 次几何查询；二分额外最多 18 次。超过 1536 步的区间恢复 Original。可提前在第一个不可见样本处拒绝。这是有明确预算的采样验证，不能证明任意小于约 4.17ms 的显隐事件不会落在采样之间，也不是多段或动态 flow 求解。

`prpr/src/note_visual.rs` 由生产 Note 渲染和求解采样共同使用，统一 Note 旋转、scale、control size/pos、incline 与 flow 变换，以及 line 旋转/平移和判定前 cover 规则。line.object.scale 只影响 carrier 自身绘制，不能乘到 Note 上。求解端另对 Hold 实际 head/body/tail 三个四边形分别判断，反向且未绘制的 body 不作为两端之间的可见包围盒。

仍未共享的是生产绘制提交、纹理尺寸/UV 切片、完整负 alpha/beat appearance 扩展和最终 camera/postprocessing。求解遇到 beat appearance 和尚未支持的负 alpha 模式时保守回退。透明像素、shader、遮挡、noise-area、HUD 和按住 Hold 之后的可读性不在区间几何验证内；实际生产 renderer 的独立审计继续作为回归出口。

`audit_preview_solver` 通过与缓存重建相同的 `note_plan` 返回 `plan`、`flow` 和 `reason`。`constant` 的 reason 保持 `solved`，`gate_only` 是受支持计划；只有 `original` 是原样回退。历史 min/max 几何诊断字段保留，不能再单独用 `reason != solved` 认定失败。
