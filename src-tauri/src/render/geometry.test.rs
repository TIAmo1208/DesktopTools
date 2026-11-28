//! `render::geometry` 的纯逻辑单测：换算、命中、钳制。

use super::geometry::{DpiScale, HostGeometry, PixelRect, dip_rect_to_pixels, hit_test_topmost, pixel_rect_to_dip};
use super::draw::Rect;

/// 125% 缩放：DIP 矩形按四边向外取整，不会把内容裁掉。
#[test]
fn dip_rect_rounds_outward_at_fractional_scale()
{
    let dpi = DpiScale::new(120); // 125%
    assert_eq!(dpi.scale(), 1.25);
    let pixels = dip_rect_to_pixels(&Rect::new(10.0, 20.0, 100.0, 50.0), dpi);
    assert_eq!(pixels, PixelRect::new(12, 25, 138, 88)); // 12.5→12、25→25、137.5→138、87.5→88
    assert_eq!(pixels.width(), 126);
    assert_eq!(pixels.height(), 63);
}

/// 150% 与 200% 下同一 DIP 矩形的像素结果。
#[test]
fn dip_rect_scales_at_150_and_200_percent()
{
    let rect = Rect::new(0.0, 0.0, 320.0, 200.0);
    assert_eq!(dip_rect_to_pixels(&rect, DpiScale::new(96)), PixelRect::new(0, 0, 320, 200));
    assert_eq!(dip_rect_to_pixels(&rect, DpiScale::new(144)), PixelRect::new(0, 0, 480, 300));
    assert_eq!(dip_rect_to_pixels(&rect, DpiScale::new(192)), PixelRect::new(0, 0, 640, 400));
}

/// 像素 → DIP → 像素应是幂等的，且四条边各自误差不超过一个像素（向外汇总是故意多画）。
#[test]
fn pixel_dip_round_trip_stays_within_one_pixel()
{
    let dpi = DpiScale::from_scale(1.5);
    let source = Rect::new(12.5, 8.0, 100.25, 40.5);
    let pixels = dip_rect_to_pixels(&source, dpi);
    let back = pixel_rect_to_dip(&pixels, dpi);
    // 再转一次必须回到同一组像素，否则「向外取整」会在来回转换中漂移。
    assert_eq!(dip_rect_to_pixels(&back, dpi), pixels);
    let scale = dpi.scale();
    assert!(((back.x - source.x) * scale).abs() <= 1.0, "左边 {} → {}", source.x, back.x);
    assert!(((back.y - source.y) * scale).abs() <= 1.0, "上边 {} → {}", source.y, back.y);
    assert!(((back.right() - source.right()) * scale).abs() <= 1.0, "右边 {} → {}", source.right(), back.right());
    assert!(((back.bottom() - source.bottom()) * scale).abs() <= 1.0, "下边 {} → {}", source.bottom(), back.bottom());
}

/// 长度换算：DIP 长度四舍五入，正数至少一像素。
#[test]
fn length_conversion_keeps_subpixel_visible()
{
    let dpi = DpiScale::new(120);
    assert_eq!(dpi.dip_to_px(1.0), 1); // 1.25 → 1，但不归零
    assert_eq!(dpi.dip_to_px(0.2), 1); // 0.25 → 至少 1
    assert_eq!(dpi.dip_to_px(8.0), 10);
    assert_eq!(dpi.dip_to_px(0.0), 0);
    assert!((dpi.px_to_dip(15) - 12.0).abs() < 0.001);
    assert_eq!(dpi.dip_point_to_px(10.0, 6.0), (13, 8)); // 12.5→13、7.5→8
    assert_eq!(dpi.px_point_to_dip(15, 9), (12.0, 7.2));
}

/// 包含判定左闭右开：右边界与下边界上的点不算命中。
#[test]
fn contains_is_half_open()
{
    let rect = PixelRect::new(100, 50, 200, 150);
    assert!(rect.contains(100, 50));
    assert!(rect.contains(199, 149));
    assert!(!rect.contains(200, 100));
    assert!(!rect.contains(100, 150));
    assert!(!rect.contains(99, 100));
}

/// 命中取最上层（数组末尾）；不命中返回 None。
#[test]
fn hit_test_prefers_topmost()
{
    let rects = [PixelRect::new(0, 0, 100, 100), PixelRect::new(50, 50, 150, 150), PixelRect::new(200, 200, 300, 300)];
    assert_eq!(hit_test_topmost(&rects, 10, 10).map(|hit| hit.index), Some(0));
    assert_eq!(hit_test_topmost(&rects, 75, 75).map(|hit| hit.index), Some(1));
    assert_eq!(hit_test_topmost(&rects, 250, 250).map(|hit| hit.index), Some(2));
    assert_eq!(hit_test_topmost(&rects, 175, 175), None);
}

/// 工作区钳制：正常平移、超出右边与下边、内层比外层大。
#[test]
fn clamp_keeps_rect_inside_work_area()
{
    let work = PixelRect::new(0, 0, 1920, 1040);
    assert_eq!(PixelRect::from_xywh(100, 100, 300, 200).clamp_into(&work), PixelRect::from_xywh(100, 100, 300, 200));
    assert_eq!(PixelRect::from_xywh(1800, 1000, 300, 200).clamp_into(&work), PixelRect::from_xywh(1620, 840, 300, 200));
    assert_eq!(PixelRect::from_xywh(-50, -20, 300, 200).clamp_into(&work), PixelRect::from_xywh(0, 0, 300, 200));
    assert_eq!(PixelRect::from_xywh(0, 0, 5000, 5000).clamp_into(&work), PixelRect::from_xywh(0, 0, 1920, 1040));
}

/// 宿主几何：表面尺寸用物理像素，DIP 尺寸供布局，工作区原点不在 (0,0) 时也要正确。
#[test]
fn host_geometry_reports_both_pixel_and_dip_sizes()
{
    let work = PixelRect::new(1920, 0, 3840, 1040); // 第二块 100% 显示器
    let geometry = HostGeometry::for_work_area(work, DpiScale::new(96));
    assert_eq!(geometry.surface_size(), (1920, 1040));
    assert_eq!(geometry.dip_size(), (1920.0, 1040.0));
    assert!(geometry.contains(1920, 0));
    assert!(!geometry.contains(0, 0));

    let scaled = HostGeometry::for_work_area(PixelRect::new(0, 0, 2560, 1400), DpiScale::new(144));
    assert_eq!(scaled.surface_size(), (2560, 1400));
    assert_eq!(scaled.dip_size().0, 1706.666_6);
}

/// 空矩形与负尺寸不会产生负数宽高。
#[test]
fn degenerate_rects_stay_non_negative()
{
    let rect = PixelRect::new(10, 10, 5, 5);
    assert_eq!(rect.width(), 0);
    assert_eq!(rect.height(), 0);
    assert!(!rect.contains(6, 6));
    assert_eq!(PixelRect::default().width(), 0);
}
