#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub left: i32,
    pub right: i32,
}

pub fn anchor(left: bool, tray_start: i32, width: i32, margin: i32) -> i32 {
    if left {
        margin
    } else {
        tray_start.saturating_sub(width).saturating_sub(margin)
    }
}
pub fn manual(anchor: i32, offset: i32, max_x: i32) -> i32 {
    anchor.saturating_add(offset).clamp(0, max_x.max(0))
}
pub fn choose(
    total: i32,
    width: i32,
    margin: i32,
    occupied: &[Interval],
    preferred: i32,
) -> Option<i32> {
    if width <= 0 || total < width + margin * 2 {
        return None;
    }
    let mut blocked: Vec<_> = occupied
        .iter()
        .filter(|i| i.right > i.left)
        .map(|i| Interval {
            left: i.left.saturating_sub(margin).clamp(margin, total - margin),
            right: i.right.saturating_add(margin).clamp(margin, total - margin),
        })
        .filter(|i| i.right > i.left)
        .collect();
    blocked.sort_by_key(|i| i.left);
    let mut gaps = Vec::new();
    let mut end = margin;
    for interval in blocked {
        if interval.left - end >= width {
            gaps.push((end, interval.left - width));
        }
        end = end.max(interval.right);
    }
    if total - margin - end >= width {
        gaps.push((end, total - margin - width));
    }
    gaps.into_iter()
        .map(|(a, b)| preferred.clamp(a, b))
        .min_by_key(|&x| ((i64::from(x) - i64::from(preferred)).abs(), x))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anchors_and_manual_bounds() {
        assert_eq!(anchor(false, 900, 124, 8), 768);
        assert_eq!(manual(anchor(true, 900, 124, 8), -200, 876), 0);
        assert_eq!(manual(768, i32::MAX, 876), 876);
    }
    #[test]
    fn returns_to_preferred_side_when_a_blocking_panel_leaves() {
        let occupied = [
            Interval {
                left: 350,
                right: 650,
            },
            Interval {
                left: 900,
                right: 1000,
            },
        ];
        let mut blocked = occupied.to_vec();
        blocked.push(Interval {
            left: 655,
            right: 900,
        });
        assert_eq!(choose(1000, 124, 8, &blocked, 768), Some(218));
        assert_eq!(choose(1000, 124, 8, &occupied, 768), Some(768));
        assert_eq!(choose(1000, 124, 8, &occupied, 8), Some(8));
    }
    #[test]
    fn no_space_and_dpi_scaled_gaps() {
        assert_eq!(
            choose(
                200,
                124,
                8,
                &[Interval {
                    left: 20,
                    right: 190
                }],
                8
            ),
            None
        );
        assert_eq!(
            choose(
                1500,
                186,
                12,
                &[
                    Interval {
                        left: 525,
                        right: 975
                    },
                    Interval {
                        left: 1350,
                        right: 1500
                    }
                ],
                1152
            ),
            Some(1152)
        );
        assert_eq!(choose(100, 124, 8, &[], 8), None);
    }
}
