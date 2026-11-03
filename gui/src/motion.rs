//! 动效规范（design/drift-motion.svg）× transitions.dev 纪律。
//!
//! 四档时长、三条曲线是全应用唯一基准（冲突时设计稿赢）：
//! 只动 transform 与 opacity；时长上限 320ms；循环进度不计入；
//! prefers-reduced-motion 时位移降为纯淡入（Motion 自动遵守，transition 需手动分支）。

use nsh_core::prelude::{
    AnimatedValue, Easing, Keyframe, KeyframeTiming, MotionChannel, MotionSpec, animated_value,
};

/// 按下 · 悬停 · 勾选描边
pub const D_MICRO: f32 = 120.0;
/// 气泡 · 标签 · 开关
pub const D_CONTROL: f32 = 180.0;
/// 记录插入 · 面板展开
pub const D_PANEL: f32 = 240.0;
/// 配对成功 · 首屏进入
pub const D_PAGE: f32 = 320.0;
/// 退场统一比进场快一档（transitions.dev 开/关不对称）
pub const D_EXIT: f32 = 160.0;
/// 逐项 stagger 间隔（texts reveal 等）
pub const D_STAGGER: f32 = 40.0;

pub type Bezier = (f32, f32, f32, f32);

/// 标准：状态切换 · 布局变化
pub const EASE_STD: Bezier = (0.2, 0.0, 0.0, 1.0);
/// 进场：气泡 · 面板 · 新记录
pub const EASE_ENTER: Bezier = (0.05, 0.7, 0.1, 1.0);
/// 退场：关闭 · 删除 · 收起
pub const EASE_EXIT: Bezier = (0.3, 0.0, 0.8, 0.15);

pub fn ease(b: Bezier) -> Easing {
    Easing::Bezier(b.0, b.1, b.2, b.3)
}

/// 新记录进入：translateY 8px→0 + 淡入，240ms · 标准。
/// 已有记录不位移（配合 animate_layout）。
pub fn enter_record() -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(D_PANEL).easing(ease(EASE_STD)))
        .values(MotionChannel::Opacity, [0.0, 1.0])
        .values(MotionChannel::TranslateY, [8.0, 0.0])
}

/// 面板/配对面板展开：scale .96→1 + 淡入，180ms · 进场。
pub fn enter_panel() -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(D_CONTROL).easing(ease(EASE_ENTER)))
        .values(MotionChannel::Opacity, [0.0, 1.0])
        .values(MotionChannel::Scale, [0.96, 1.0])
}

/// 通用进场：上移淡入（空态文案、建议条），可加 stagger 延迟。
pub fn enter_rise(delay_ms: f32) -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(D_PANEL).easing(ease(EASE_ENTER)).delay(delay_ms))
        .values(MotionChannel::Opacity, [0.0, 1.0])
        .values(MotionChannel::TranslateY, [12.0, 0.0])
}

/// 退场：缩小淡出，160ms · 退场（播放完毕再卸载节点）。
pub fn exit_fade() -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(D_EXIT).easing(ease(EASE_EXIT)))
        .values(MotionChannel::Opacity, [1.0, 0.0])
        .values(MotionChannel::Scale, [1.0, 0.97])
}

/// 点按设备发送（已并入 orb_press_value：按下 .94 · 120ms / 回弹 100ms）。
pub fn device_tap_value() -> AnimatedValue {
    orb_press_value()
}

/// 配对成功：对勾弹入（scale 0.6→1.08→1 + 淡入，320ms）。
pub fn success_check() -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(D_PAGE).easing(ease(EASE_ENTER)))
        .track(
            MotionChannel::Scale,
            [
                Keyframe::at(0.0, 0.6).ease(ease(EASE_ENTER)),
                Keyframe::at(0.7, 1.08),
                Keyframe::at(1.0, 1.0),
            ],
        )
        .values(MotionChannel::Opacity, [0.0, 1.0])
}

/// 配对不一致：横向微幅抖动（120ms · 每段），error-shake 语义。
pub fn mismatch_shake() -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(D_MICRO).easing(Easing::Linear))
        .values(MotionChannel::TranslateX, [0.0, -5.0, 5.0, -3.0, 0.0])
}

// —— 设备球体画布（drift-motion 场景 1 / 2 / 4）——

/// 球体随输入浮出：translateY(18→0) + 淡入，180ms · 进场，逐球 40ms 错开。
pub fn orb_rise(delay_ms: f32) -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(D_CONTROL).easing(ease(EASE_ENTER)).delay(delay_ms))
        .values(MotionChannel::Opacity, [0.0, 1.0])
        .values(MotionChannel::TranslateY, [18.0, 0.0])
}

/// 新球从加号球位置展开：scale .6→1 + 淡入，240ms · 标准（环同步出现）。
pub fn orb_spawn(delay_ms: f32) -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(D_PANEL).easing(ease(EASE_ENTER)).delay(delay_ms))
        .values(MotionChannel::Opacity, [0.0, 1.0])
        .values(MotionChannel::Scale, [0.6, 1.0])
}

/// 球体按下：scale .94（120ms），松手回弹（100ms）。
/// 返回 animated value，用 `bind_scale` 绑到球体。
pub fn orb_press_value() -> AnimatedValue {
    let v = animated_value(1.0);
    v.keyframes(
        [
            Keyframe::new(1.0),
            Keyframe::at(0.55, 0.94).ease(ease(EASE_STD)),
            Keyframe::new(1.0),
        ],
        KeyframeTiming::new(220.0),
    );
    v
}

/// 球体环境浮动循环：7s 缓升缓降 -6px，`phase` 错开各球相位。
/// 只在未开启「减少动态」时使用；循环不计入时长上限。
pub fn orb_bob_value(phase_ms: f32) -> AnimatedValue {
    let v = animated_value(0.0);
    v.keyframes(
        [
            Keyframe::at(0.0, 0.0).ease(Easing::EaseInOut),
            Keyframe::at(0.5, -6.0).ease(Easing::EaseInOut),
            Keyframe::at(1.0, 0.0),
        ],
        KeyframeTiming::new(7000.0).iterations(None).delay(phase_ms),
    );
    v
}

/// 撤销条滑入：上移淡入，180ms · 进场（5s 后按 D_EXIT 退出）。
pub fn undo_enter() -> MotionSpec {
    MotionSpec::new(KeyframeTiming::new(D_CONTROL).easing(ease(EASE_ENTER)))
        .values(MotionChannel::Opacity, [0.0, 1.0])
        .values(MotionChannel::TranslateY, [10.0, 0.0])
}
