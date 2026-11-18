//! 遮罩合并与区域排除（T02.3，架构 §8.2 / §4 #44）。
//!
//! # 两道防线里的第二道（§1.3）
//!
//! 采样区里压着我们自己改色的时钟/日期文字（EX-28）。第一道防线是 P50 鲁棒统计量，
//! 第二道就是在直方图单遍扫描里**直接跳过**这些像素。本模块提供：
//!
//! * 固定容量 [`MaskSet`]（零堆分配，容量 [`MAX_MASKS`]）；
//! * [`MaskSet::union_with`] 合并 UIA 矩形 + DLL 回传的 `ColoredRects`；
//! * 每个输入矩形自动 [`MASK_INFLATE_PX`] 膨胀 2 px，覆盖文字抗锯齿边缘；
//! * [`MaskSet::hits`] 命中测试与 [`MaskSet::valid_px`] 有效像素统计。
//!
//! 所有坐标都是**物理像素**（§11.4 铁律）。

use crate::engine::{
    MAX_MASKS, MASK_INFLATE_PX, RectPx, rect_area, rect_contains, rect_inflate, rect_intersection,
    rect_is_empty, rect_px, rect_touches, rect_union,
};

/// 遮罩来源，仅用于日志与诊断（不影响命中逻辑）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaskKind {
    /// UIA 定位到的时钟文字矩形。
    #[default]
    Clock,
    /// UIA 定位到的日期文字矩形。
    Date,
    /// L3：悬停块。
    Hover,
    /// L3：运行指示条。
    Indicator,
    /// DLL 回传的已着色矩形（`ColoredRects` 帧）。
    ColoredRect,
    /// 置顶窗口遮挡（EX-24）。
    Occlusion,
}

/// 一个遮罩矩形（物理像素）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaskRect {
    /// 矩形（未膨胀前的原始屏幕坐标，px）。
    pub rect: RectPx,
    /// 来源。
    pub kind: MaskKind,
}

impl Default for MaskRect {
    fn default() -> Self {
        Self {
            rect: rect_px(0, 0, 0, 0),
            kind: MaskKind::Clock,
        }
    }
}

impl MaskRect {
    /// 构造。
    #[must_use]
    pub const fn new(rect: RectPx, kind: MaskKind) -> Self {
        Self { rect, kind }
    }
}

/// 遮罩集合：固定容量数组 + 有效长度，**永不堆分配**。
///
/// 合并策略：相交或相邻（`gap = 1 px`）的矩形反复合并成包围矩形，减少命中测试的
/// 比较次数；容量用尽时把多余矩形并入最后一个（宁可多遮、不可漏遮，避免 A-28 翻转）。
pub struct MaskSet {
    rects: [MaskRect; MAX_MASKS],
    len: usize,
    overflow: u64,
}

impl Default for MaskSet {
    fn default() -> Self {
        Self::new()
    }
}

impl MaskSet {
    /// 空集合。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            rects: [MaskRect::new(rect_px(0, 0, 0, 0), MaskKind::Clock); MAX_MASKS],
            len: 0,
            overflow: 0,
        }
    }

    /// 清空（保留容量，不释放内存）。
    pub fn clear(&mut self) {
        self.len = 0;
        self.overflow = 0;
    }

    /// 当前矩形数量。
    #[must_use]
    pub const fn count(&self) -> usize {
        self.len
    }

    /// 容量溢出累计次数（诊断用；>0 说明应调大 [`MAX_MASKS`]）。
    #[must_use]
    pub const fn overflow(&self) -> u64 {
        self.overflow
    }

    /// 有效矩形切片。
    #[must_use]
    pub fn rects(&self) -> &[MaskRect] {
        &self.rects[..self.len]
    }

    /// 合并一批新矩形。
    ///
    /// 每个矩形先膨胀 [`MASK_INFLATE_PX`]，再与已有矩形做连通合并。空矩形直接忽略。
    /// 重复调用（UIA 每 5 s 刷一次）不会累积出无限多矩形。
    pub fn union_with(&mut self, incoming: &[MaskRect]) {
        for &item in incoming {
            // 先在**原始坐标**上过滤退化/反演矩形：若等膨胀后再判断，
            // 一个宽度为 0 的矩形会被 2 px 膨胀"救活"，把不存在的遮挡算进统计。
            if rect_is_empty(&item.rect) {
                continue;
            }
            let inflated = MaskRect::new(rect_inflate(&item.rect, MASK_INFLATE_PX), item.kind);
            if rect_is_empty(&inflated.rect) {
                continue;
            }
            self.insert_merging(inflated);
        }
    }

    /// 插入并反复合并，直到与所有已有矩形都不再相邻。
    fn insert_merging(&mut self, mut candidate: MaskRect) {
        let mut i = 0usize;
        while i < self.len {
            if rect_touches(&self.rects[i].rect, &candidate.rect, 1) {
                candidate.rect = rect_union(&self.rects[i].rect, &candidate.rect);
                // 保持 kind 为更"重要"的一侧：文字类优先于装饰类。
                if matches!(self.rects[i].kind, MaskKind::Clock | MaskKind::Date) {
                    candidate.kind = self.rects[i].kind;
                }
                self.remove_at(i);
                i = 0;
            } else {
                i += 1;
            }
        }
        if self.len < MAX_MASKS {
            self.rects[self.len] = candidate;
            self.len += 1;
        } else {
            self.overflow += 1;
            let last = MAX_MASKS - 1;
            self.rects[last].rect = rect_union(&self.rects[last].rect, &candidate.rect);
        }
    }

    /// 删除下标处的矩形（保持顺序）。
    fn remove_at(&mut self, index: usize) {
        if index >= self.len {
            return;
        }
        let mut j = index;
        while j + 1 < self.len {
            self.rects[j] = self.rects[j + 1];
            j += 1;
        }
        self.len -= 1;
    }

    /// 点 `(x, y)` 是否落在任一遮罩内（半开区间 `[left, right) × [top, bottom)`）。
    #[must_use]
    pub fn hits(&self, x: i32, y: i32) -> bool {
        for r in self.rects() {
            if x >= r.rect.left
                && x < r.rect.right
                && y >= r.rect.top
                && y < r.rect.bottom
            {
                return true;
            }
        }
        false
    }

    /// 采样矩形内**被遮罩覆盖**的像素数。
    ///
    /// 由于集合内矩形互不相交（合并保证），直接累加交集面积不会重复计数。
    #[must_use]
    pub fn covered_px(&self, sample: &RectPx) -> u32 {
        let mut total = 0u32;
        for r in self.rects() {
            if let Some(part) = rect_intersection(sample, &r.rect) {
                total = total.saturating_add(rect_area(&part));
            }
        }
        total.min(rect_area(sample))
    }

    /// 采样矩形内**参与统计**的有效像素数（遮罩后）。
    ///
    /// 这是 §13-5 `valid_px < 1500` 判定的唯一来源。
    #[must_use]
    pub fn valid_px(&self, sample: &RectPx) -> u32 {
        rect_area(sample).saturating_sub(self.covered_px(sample))
    }

    /// 采样区是否被遮罩 100% 覆盖（窗口遮挡 / 托盘折叠到极小 / 遮罩配置异常）。
    #[must_use]
    pub fn is_fully_covered(&self, sample: &RectPx) -> bool {
        self.valid_px(sample) == 0
    }

    /// 采样矩形是否完整落在某个遮罩内（诊断用）。
    #[must_use]
    pub fn sample_is_inside_mask(&self, sample: &RectPx) -> bool {
        self.rects()
            .iter()
            .any(|r| rect_contains(&r.rect, sample))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::{alloc_count, reset_alloc_count};

    fn clock_rect() -> MaskRect {
        MaskRect::new(rect_px(1700, 1036, 1830, 1060), MaskKind::Clock)
    }
    fn date_rect() -> MaskRect {
        MaskRect::new(rect_px(1700, 1060, 1830, 1080), MaskKind::Date)
    }

    #[test]
    fn empty_set_hits_nothing_and_covers_nothing() {
        let m = MaskSet::new();
        assert_eq!(m.count(), 0);
        assert!(!m.hits(10, 10));
        let s = rect_px(0, 0, 300, 48);
        assert_eq!(m.covered_px(&s), 0);
        assert_eq!(m.valid_px(&s), 300 * 48);
        assert!(!m.is_fully_covered(&s));
    }

    #[test]
    fn union_inflates_by_two_pixels() {
        let mut m = MaskSet::new();
        m.union_with(&[clock_rect()]);
        assert_eq!(m.count(), 1);
        assert_eq!(
            m.rects()[0].rect,
            rect_px(1698, 1034, 1832, 1062),
            "T02.3 判据①：膨胀 2 px"
        );
        assert!(m.hits(1699, 1035));
        assert!(!m.hits(1697, 1035));
    }

    #[test]
    fn adjacent_clock_and_date_merge_into_one() {
        let mut m = MaskSet::new();
        m.union_with(&[clock_rect(), date_rect()]);
        assert_eq!(m.count(), 1, "相邻矩形应连通合并");
        assert_eq!(m.rects()[0].rect, rect_px(1698, 1034, 1832, 1082));
        assert_eq!(m.rects()[0].kind, MaskKind::Clock, "文字类 kind 优先保留");
    }

    #[test]
    fn disjoint_rects_stay_separate_and_areas_do_not_double_count() {
        let mut m = MaskSet::new();
        m.union_with(&[
            MaskRect::new(rect_px(10, 10, 20, 20), MaskKind::Clock),
            MaskRect::new(rect_px(100, 100, 110, 110), MaskKind::Date),
        ]);
        assert_eq!(m.count(), 2);
        let sample = rect_px(0, 0, 200, 200);
        // 每个膨胀后为 14×14 = 196，两个共 392。
        assert_eq!(m.covered_px(&sample), 392);
        assert_eq!(m.valid_px(&sample), 200 * 200 - 392);
    }

    #[test]
    fn duplicate_union_does_not_grow_without_bound() {
        let mut m = MaskSet::new();
        for _ in 0..100 {
            m.union_with(&[clock_rect(), date_rect()]);
        }
        assert_eq!(m.count(), 1, "重复刷新不得累积矩形");
        assert_eq!(m.overflow(), 0);
    }

    #[test]
    fn fully_masked_region_is_detected() {
        let mut m = MaskSet::new();
        let sample = rect_px(100, 100, 200, 200);
        m.union_with(&[MaskRect::new(sample, MaskKind::Occlusion)]);
        assert!(m.is_fully_covered(&sample), "采样区被全遮罩覆盖");
        assert_eq!(m.valid_px(&sample), 0);
        assert!(m.sample_is_inside_mask(&sample));
    }

    #[test]
    fn topmost_window_occlusion_is_excluded() {
        // EX-24：一个覆盖采样区上半部的置顶窗口 → 有效像素恰好减半。
        let sample = rect_px(0, 0, 100, 100);
        let mut m = MaskSet::new();
        m.union_with(&[MaskRect::new(rect_px(0, 0, 100, 48), MaskKind::Occlusion)]);
        assert!(m.covered_px(&sample) >= 100 * 48);
        assert!(m.valid_px(&sample) <= 100 * 52);
        assert!(!m.is_fully_covered(&sample), "只遮一半时仍可采样");
    }

    #[test]
    fn capacity_overflow_never_drops_a_mask() {
        let mut m = MaskSet::new();
        let mut many = Vec::new();
        for i in 0..(MAX_MASKS + 8) {
            let x = 1000 + i * 10;
            many.push(MaskRect::new(
                rect_px(x as i32, 0, x as i32 + 1, 1),
                MaskKind::ColoredRect,
            ));
        }
        m.union_with(&many);
        assert_eq!(m.count(), MAX_MASKS);
        assert!(m.overflow() > 0);
        // 最后一个矩形被并入，最右侧的点仍应命中。
        let rightmost = (1000 + (MAX_MASKS + 7) * 10) as i32;
        assert!(m.hits(rightmost + 2, 2), "容量满时也不得漏遮");
    }

    #[test]
    fn hits_and_union_are_allocation_free() {
        let mut m = MaskSet::new();
        m.union_with(&[clock_rect(), date_rect()]);
        reset_alloc_count();
        for _ in 0..600 {
            let mut acc = 0u32;
            for y in 0..48 {
                for x in 0..300 {
                    if !m.hits(x, y) {
                        acc += 1;
                    }
                }
            }
            assert!(acc > 0);
            let _ = m.valid_px(&rect_px(0, 0, 300, 48));
        }
        assert_eq!(alloc_count(), 0, "热路径命中测试不得分配");
    }

    #[test]
    fn empty_and_inverted_rects_are_ignored() {
        let mut m = MaskSet::new();
        m.union_with(&[
            MaskRect::new(rect_px(10, 10, 10, 20), MaskKind::Clock),
            MaskRect::new(rect_px(30, 30, 20, 10), MaskKind::Date),
        ]);
        assert_eq!(m.count(), 0);
    }
}
