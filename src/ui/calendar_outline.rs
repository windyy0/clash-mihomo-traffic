use std::collections::BTreeMap;

// Date fills are 28 DIP with a 4 DIP gutter. Selection edges occupy the
// gutter midpoint; only disconnected, wrapped short ranges need a thin bridge.
pub(super) fn selection_contours(bands: &[(usize, usize, usize)]) -> Vec<Vec<(f32, f32)>> {
    let mut rects: Vec<_> = bands
        .iter()
        .map(|&(row, first, last)| {
            (
                first as f32 * 32. - 2.,
                row as f32 * 32. - 2.,
                (last + 1) as f32 * 32. - 2.,
                (row + 1) as f32 * 32. - 2.,
            )
        })
        .collect();
    for pair in bands.windows(2) {
        let (a, first_a, last_a) = pair[0];
        let (b, first_b, last_b) = pair[1];
        if b == a + 1 && (first_a > last_b || first_b > last_a) {
            let center = b as f32 * 32. - 2.;
            rects.push((
                first_a.min(first_b) as f32 * 32. - 2.,
                center - 1.,
                (last_a.max(last_b) + 1) as f32 * 32. - 2.,
                center + 1.,
            ));
        }
    }
    if rects.is_empty() {
        return Vec::new();
    }
    let mut xs: Vec<_> = rects.iter().flat_map(|r| [r.0, r.2]).collect();
    let mut ys: Vec<_> = rects.iter().flat_map(|r| [r.1, r.3]).collect();
    xs.sort_by(f32::total_cmp);
    xs.dedup();
    ys.sort_by(f32::total_cmp);
    ys.dedup();
    let cells: Vec<Vec<_>> = ys
        .windows(2)
        .map(|y| {
            xs.windows(2)
                .map(|x| {
                    let (cx, cy) = ((x[0] + x[1]) / 2., (y[0] + y[1]) / 2.);
                    rects
                        .iter()
                        .any(|r| cx > r.0 && cx < r.2 && cy > r.1 && cy < r.3)
                })
                .collect()
        })
        .collect();
    let mut edges = BTreeMap::new();
    for (y, row) in cells.iter().enumerate() {
        for (x, &filled) in row.iter().enumerate() {
            if !filled {
                continue;
            }
            if y == 0 || !cells[y - 1][x] {
                edges.insert((x, y), (x + 1, y));
            }
            if x + 1 == row.len() || !row[x + 1] {
                edges.insert((x + 1, y), (x + 1, y + 1));
            }
            if y + 1 == cells.len() || !cells[y + 1][x] {
                edges.insert((x + 1, y + 1), (x, y + 1));
            }
            if x == 0 || !row[x - 1] {
                edges.insert((x, y + 1), (x, y));
            }
        }
    }
    let mut result = Vec::new();
    while let Some((&start, _)) = edges.first_key_value() {
        let mut path = vec![start];
        let mut at = start;
        while let Some(next) = edges.remove(&at) {
            if next == start {
                break;
            }
            path.push(next);
            at = next;
        }
        result.push(
            path.iter()
                .enumerate()
                .filter_map(|(i, &p)| {
                    let a = path[(i + path.len() - 1) % path.len()];
                    let b = path[(i + 1) % path.len()];
                    ((a.0 != p.0 || p.0 != b.0) && (a.1 != p.1 || p.1 != b.1))
                        .then_some((xs[p.0], ys[p.1]))
                })
                .collect(),
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::selection_contours;
    fn contains(path: &[(f32, f32)], x: f32, y: f32) -> bool {
        let mut inside = false;
        for (i, &(ax, ay)) in path.iter().enumerate() {
            let (bx, by) = path[(i + 1) % path.len()];
            if (ay > y) != (by > y) && x < (bx - ax) * (y - ay) / (by - ay) + ax {
                inside = !inside;
            }
        }
        inside
    }
    #[test]
    fn every_range_is_connected_and_excludes_unselected_date_centers() {
        for first in 0usize..42 {
            for last in first..42 {
                let bands: Vec<_> = (first / 7..=last / 7)
                    .map(|row| (row, first.saturating_sub(row * 7), (last - row * 7).min(6)))
                    .collect();
                let paths = selection_contours(&bands);
                assert_eq!(paths.len(), 1, "range {first}..{last}");
                for d in 0..42 {
                    assert_eq!(
                        contains(
                            &paths[0],
                            (d % 7) as f32 * 32. + 14.,
                            (d / 7) as f32 * 32. + 14.
                        ),
                        (first..=last).contains(&d),
                        "range {first}..{last}, date {d}"
                    );
                }
            }
        }
    }
    #[test]
    fn screenshot_step_uses_the_middle_of_the_gutter() {
        let paths = selection_contours(&[(0, 2, 6), (1, 0, 6), (2, 0, 3)]);
        assert!(paths[0].contains(&(-2., 30.)));
        assert!(paths[0].contains(&(62., 30.)));
        assert!(paths[0].iter().all(|&(_, y)| (y + 2.) % 32. == 0.));
    }
    #[test]
    fn full_grid_has_four_corners_and_empty_range_has_none() {
        let bands: Vec<_> = (0..53).map(|row| (row, 0, 6)).collect();
        assert_eq!(selection_contours(&bands)[0].len(), 4);
        assert!(selection_contours(&[]).is_empty());
    }
}
