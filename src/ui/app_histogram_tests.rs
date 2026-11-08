use super::*;

fn bounds(width: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(13.), px(17.)), size(px(width), px(120.)))
}
fn histogram() -> Histogram {
    let day = 86_400_000;
    Histogram {
        size: BucketSize::Month,
        buckets: vec![
            Bucket {
                from: 0,
                to: 31 * day,
                bytes: Bytes { up: 50, down: 100 },
            },
            Bucket {
                from: 31 * day,
                to: 59 * day,
                bytes: Bytes { up: 20, down: 40 },
            },
        ],
    }
}
#[::core::prelude::v1::test]
fn total_curve_uses_the_stacked_volume_axis_and_equal_width_calendar_centers() {
    let h = histogram();
    for width in [120., 308.] {
        let area = bounds(width);
        let points = total_controls(&h, area, 300.);
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].x, area.left() + area.size.width * 0.25);
        assert_eq!(points[1].x, area.left() + area.size.width * 0.75);
        assert_eq!(points[0].y, area.bottom() - area.size.height * 0.5);
        assert_eq!(points[1].y, area.bottom() - area.size.height * 0.2);
        let smooth = chart::smooth_points(&points);
        assert_eq!(smooth.first(), points.first());
        assert_eq!(smooth.last(), points.last());
        assert!(smooth.iter().all(|p| p.x >= area.left()
            && p.x <= area.right()
            && p.y >= area.top()
            && p.y <= area.bottom()));
    }
}
#[::core::prelude::v1::test]
fn total_curve_handles_empty_zero_single_bucket_and_large_amounts() {
    let area = bounds(308.);
    assert!(total_controls(&Histogram::default(), area, 1024.).is_empty());
    let mut h = histogram();
    h.buckets.truncate(1);
    h.buckets[0].bytes = Bytes::default();
    let points = total_controls(&h, area, 1024.);
    assert_eq!(
        points,
        vec![point(area.left() + area.size.width / 2., area.bottom())]
    );
    assert_eq!(chart::smooth_points(&points), points);
    h.buckets[0].bytes = Bytes {
        down: u64::MAX,
        up: u64::MAX,
    };
    let points = total_controls(&h, area, u64::MAX as f64 * 2.);
    assert_eq!(points[0].y, area.top());
}
#[::core::prelude::v1::test]
fn hover_preserves_exact_cross_day_interval_and_download_upload_total() {
    let local = |day, hour| {
        chrono::NaiveDate::from_ymd_opt(2024, 2, day)
            .unwrap()
            .and_hms_opt(hour, 30, 0)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .earliest()
            .unwrap()
            .timestamp_millis()
    };
    let b = Bucket {
        from: local(28, 23),
        to: local(29, 0),
        bytes: Bytes {
            down: 2048,
            up: 512,
        },
    };
    assert_eq!(
        bucket_tooltip(&b),
        "2024/02/28 23:30 — 2024/02/29 00:30\n下载 2.0 KB\n上传 512 B\n合计 2.5 KB\n右键查看详情"
    );
}
