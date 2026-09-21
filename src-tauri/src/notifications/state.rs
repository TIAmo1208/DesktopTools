use super::ToastRequest;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// 一条消息当前呈现的几何与独立透明度，坐标以堆栈底边为原点。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Visual
{
    pub top: f32,     // 相对底边的纵向坐标。
    pub height: f32,  // 完整消息高度。
    pub scale: f32,   // 折叠后层的缩放比例。
    pub alpha: f32,   // 消息本身透明度。
    pub content: f32, // 后层正文与图标的可见度。
}

/// 由当前画面转向目标画面的有限动画。
pub(super) struct Motion
{
    pub from: Visual,     // 过渡开始时的画面。
    pub to: Visual,       // 目标画面。
    pub started: Instant, // 本次过渡起点。
}

impl Motion
{
    /// 离场使用 ease-in，其余使用 ease-out，均为 250ms。
    pub fn sample(&self, now: Instant, closing: bool, animated: bool) -> Visual
    {
        let elapsed = if animated
        {
            (now.saturating_duration_since(self.started).as_secs_f32() / 0.25).min(1.0)
        }
        else
        {
            1.0
        }; // 归一化进度。
        let progress = if closing { elapsed * elapsed } else { 1.0 - (1.0 - elapsed).powi(3) }; // 已确认的缓动方向。
        let mix = |from: f32, to: f32| from + (to - from) * progress; // 所有属性从同一呈现帧衔接。
        Visual { top: mix(self.from.top, self.to.top), height: mix(self.from.height, self.to.height), scale: mix(self.from.scale, self.to.scale), alpha: mix(self.from.alpha, self.to.alpha), content: mix(self.from.content, self.to.content) }
    }

    /// 稳定状态不需要继续唤醒计时器。
    pub fn active(&self, now: Instant, animated: bool) -> bool { animated && self.from != self.to && now.saturating_duration_since(self.started) < Duration::from_millis(250) }
}

/// 队列中的一条消息；退出动画完成前保留身份与内容。
pub(super) struct ToastItem
{
    pub id: u64,               // 稳定身份用于文本缓存与点击命中。
    pub request: ToastRequest, // 原始消息。
    pub expires_at: Instant,   // 当前停留截止时刻。
    pub created_at: Instant,   // 悬停期间新增消息只暂停自身存在的时间。
    pub height: f32,           // DirectWrite 测量后的布局高度。
    pub closing: bool,         // 已进入离场，不再参与补位计算。
    pub motion: Motion,        // 独立的插值状态。
}

/// 单一锁保护消息与悬停状态，不嵌套获取队列锁。
pub(super) struct ToastState
{
    pub items: VecDeque<ToastItem>,     // 从旧到新的有界队列。
    pub hovered_since: Option<Instant>, // 悬停期间暂停自动关闭。
    next_id: u64,                       // 当前窗口内递增身份。
}

impl Default for ToastState
{
    /// 空队列没有活动计时器。
    fn default() -> Self { Self { items: VecDeque::new(), hovered_since: None, next_id: 0 } }
}

impl ToastState
{
    /// 按来源去重并加入消息，旧层从其当前呈现状态开始让位。
    pub fn enqueue(&mut self, request: ToastRequest, height: f32, now: Instant, animated: bool) -> bool
    {
        if request.dedupe_key.as_ref().is_some_and(|key| self.items.iter().any(|item| !item.closing && item.request.source == request.source && item.request.dedupe_key.as_ref() == Some(key)))
        {
            return false;
        }
        self.next_id += 1;
        let initial = Visual { top: -height + 16.0, height, scale: 1.0, alpha: 0.0, content: 1.0 }; // 从底部下方透明进入。
        let expires_at = now + Duration::from_millis(u64::from(request.duration_ms)); // 每条消息独立期限，不按队列顺序关闭。
        self.items.push_back(ToastItem { id: self.next_id, request, height, created_at: now, expires_at, closing: false, motion: Motion { from: initial, to: initial, started: now } });
        while self.items.len() > 5
        {
            self.items.pop_front();
        } // 沿用最多五条的队列上限。
        self.reflow(now, animated);
        true
    }

    /// 关闭单条消息，剩余层在同一帧补位，不改变整栈透明度。
    pub fn close(&mut self, id: u64, now: Instant, animated: bool)
    {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id && !item.closing)
        {
            let current = item.motion.sample(now, false, animated); // 当前可见帧。
            item.closing = true;
            item.motion = Motion { from: current, to: Visual { top: current.top + 16.0, alpha: 0.0, ..current }, started: now };
            self.reflow(now, animated);
        }
    }

    /// 新增、关闭或悬停变化时重算目标，帧刷新不重启动画。
    pub fn reflow(&mut self, now: Instant, animated: bool)
    {
        let expanded = self.hovered_since.is_some(); // 当前悬停目标。
        let newest_height = self.items.iter().rev().find(|item| !item.closing).map_or(64.0, |item| item.height); // 前景顶部是 peek 层的锚点。
        let mut depth = 0usize; // 不包括退出中的层。
        let mut bottom = 0.0; // 展开时从下向上累计真实高度。
        for item in self.items.iter_mut().rev().filter(|item| !item.closing)
        {
            let scale = if expanded { 1.0 } else { 1.0 - 0.06 * depth.min(2) as f32 }; // 100% / 94% / 88%。
            let target = Visual { top: if expanded { bottom - item.height } else { -newest_height - 14.0 * depth.min(2) as f32 }, height: if expanded { item.height } else { newest_height }, scale, alpha: if expanded || depth < 3 { 1.0 } else { 0.0 }, content: if expanded || depth == 0 { 1.0 } else { 0.0 } }; // 折叠后层沿用前景高度，长旧消息不能从底部露出。
            if target != item.motion.to
            {
                let current = item.motion.sample(now, false, animated); // 连续变化不回到旧起点。
                item.motion = Motion { from: current, to: target, started: now };
            }
            bottom -= item.height + 14.0;
            depth += 1;
        }
    }

    /// 暂停并恢复每条消息实际经历的悬停时长。
    pub fn hover(&mut self, expanded: bool, now: Instant, animated: bool)
    {
        if expanded == self.hovered_since.is_some()
        {
            return;
        }
        if expanded
        {
            self.hovered_since = Some(now);
        }
        else if let Some(started) = self.hovered_since.take()
        {
            for item in &mut self.items
            {
                item.expires_at += now.saturating_duration_since(started.max(item.created_at));
            }
        }
        self.reflow(now, animated);
    }

    /// 处理过期及完成离场，返回下一次真正需要唤醒的时间。
    pub fn tick(&mut self, now: Instant, animated: bool) -> Option<u32>
    {
        if self.hovered_since.is_none()
        {
            let expired = self.items.iter().filter(|item| !item.closing && item.expires_at <= now).map(|item| item.id).collect::<Vec<_>>(); // 锁内只处理领域状态，不调用窗口 API。
            for id in expired
            {
                self.close(id, now, animated);
            }
        }
        self.items.retain(|item| !item.closing || item.motion.active(now, animated));
        if self.items.is_empty()
        {
            self.hovered_since = None;
            return None;
        }
        if self.items.iter().any(|item| item.motion.active(now, animated))
        {
            return Some(16);
        }
        if self.hovered_since.is_some()
        {
            return (animated && self.items.iter().any(|item| item.request.loading == Some(true) && item.motion.to.content > 0.0 && item.motion.to.alpha > 0.0)).then_some(33);
        }
        let expiry = self.items.iter().filter(|item| !item.closing).map(|item| item.expires_at.saturating_duration_since(now).as_millis().clamp(1, u32::MAX as u128) as u32).min(); // 即使后入队消息先到期也独立退出。
        if animated && self.items.iter().any(|item| item.request.loading == Some(true) && item.motion.to.content > 0.0 && item.motion.to.alpha > 0.0)
        {
            Some(expiry.unwrap_or(33).min(33))
        }
        else
        {
            expiry
        }
    }
}
