use crate::util::{intersections, sweep_points_to_points};
use geo::{
    sweep::SweepPoint, Contains, Coord, CoordsIter, GeoFloat, Intersects, LineString, LinesIter,
    Point,
};
use rstar::{RTree, RTreeObject};

fn merge_two<T: GeoFloat>(a: &LineString<T>, b: &LineString<T>) -> Option<LineString<T>> {
    if a.0[0].intersects(&b.0[0]) {
        Some(LineString::from_iter(
            a.coords_iter().rev().chain(b.coords_iter().skip(1)),
        ))
    } else if a.0[a.0.len() - 1].intersects(&b.0[0]) {
        Some(LineString::from_iter(
            a.coords_iter().chain(b.coords_iter().skip(1)),
        ))
    } else if a.0[a.0.len() - 1].intersects(&b.0[b.0.len() - 1]) {
        Some(LineString::from_iter(
            a.coords_iter().chain(b.coords_iter().rev().skip(1)),
        ))
    } else if a.0[0].intersects(&b.0[b.0.len() - 1]) {
        Some(LineString::from_iter(
            b.coords_iter().chain(a.coords_iter().skip(1)),
        ))
    } else {
        None
    }
}

fn check_lines_intersect_points<T: GeoFloat>(
    lines: &Vec<&LineString<T>>,
    points: &RTree<Point<T>>,
) -> bool {
    for line in lines {
        for candidate in points.locate_in_envelope_intersecting(&line.envelope()) {
            if candidate.intersects(*line) {
                return true;
            }
        }
    }
    return false;
}

fn check_line_in_merged_lines<T: GeoFloat>(
    merged_lines: &RTree<LineString<T>>,
    line: &LineString<T>,
) -> bool {
    for candidate in merged_lines.locate_in_envelope_intersecting(&line.envelope()) {
        if candidate.contains(line) {
            return true;
        }
    }
    return false;
}

fn _get_line_endpoints<T: GeoFloat>(line: &LineString<T>) -> (&Coord<T>, &Coord<T>) {
    (line.0.first().unwrap(), line.0.last().unwrap())
}

pub fn merge_linestring_optimized<T: GeoFloat + Send + Sync>(
    lines: Vec<LineString<T>>,
) -> Vec<LineString<T>> {
    let lines = RTree::bulk_load(lines);
    let linestring_intersections = intersections::<T, SweepPoint<T>, SweepPoint<T>>(
        lines.iter().flat_map(|linestring| linestring.lines_iter()),
    );
    let mut point_intersections = Vec::new();
    point_intersections.extend(sweep_points_to_points(linestring_intersections.1 .0));
    point_intersections.extend(sweep_points_to_points(linestring_intersections.1 .1));
    let points_intersections = RTree::bulk_load(point_intersections);

    let mut merged_lines: RTree<LineString<T>> = RTree::new();

    for line in lines.iter() {
        if check_line_in_merged_lines(&merged_lines, line) {
            continue;
        }
        let mut prev_line = Some(line.clone());
        loop {
            let line = prev_line.as_ref().unwrap();
            let mut computed_line: Option<LineString<T>> = None;
            let intersecting_lines: Vec<_> = lines
                .locate_in_envelope_intersecting(&line.envelope())
                .filter(|other_line| {
                    other_line.intersects(line)
                        && other_line.ne(&line)
                        && !check_line_in_merged_lines(&merged_lines, other_line)
                })
                .collect();
            let intersecting_lines_count = intersecting_lines.len();
            println!("{intersecting_lines_count}");
            let lines_intersect_points =
                check_lines_intersect_points(&intersecting_lines, &points_intersections);
            if intersecting_lines_count == 0 {
                merged_lines.insert(line.clone());
                break;
            } else if intersecting_lines_count == 1 {
                if let Some(line) = merge_two(intersecting_lines[0], line) {
                    computed_line.replace(line);
                }
            } else if intersecting_lines_count == 2 && !lines_intersect_points {
                let first = merge_two(intersecting_lines[0], &line).unwrap();
                let second = merge_two(intersecting_lines[1], &first).unwrap();
                computed_line.replace(second);
            }
            if prev_line.eq(&computed_line) {
                break;
            } else if let Some(line) = computed_line {
                merged_lines.insert(line.clone());
                prev_line.replace(line);
            } else {
                break;
            }
        }
    }
    merged_lines.into_iter().collect()
}

#[cfg(test)]
mod tests {

    // TODO: Add tests for case where linestring is circular

    use super::*;
    use geo::line_string;

    #[test]
    fn test_one() {
        let input = vec![line_string![
            (x: 1., y: 1.),
            (x: 2., y: 2.),
        ]];
        let output = input.clone();
        assert_eq!(merge_linestring_optimized(input), output);
    }

    #[test]
    fn touches_two() {
        let input = vec![
            line_string![(x: 1., y: 1.), (x: 2., y: 2.)],
            line_string![(x: 2., y: 2.), (x: 3., y: 3.)],
        ];
        let output = merge_linestring_optimized(input.clone());
        assert!(output.contains(&line_string![
            (x: 1., y: 1.),
            (x: 2., y: 2.),
            (x: 3., y: 3.)
        ]));
    }

    #[test]
    fn touches_three() {
        let input = vec![
            line_string![(x: 1.0, y: 2.0), (x: 3.0, y: 4.0)],
            line_string![(x: 3.0, y: 4.0), (x: 5.0, y: 6.0)],
            line_string![(x: 5.0, y: 6.0), (x: 7.0, y: 8.0)],
        ];
        let output = vec![line_string![
            (x: 1.0, y: 2.0),
            (x: 3.0, y: 4.0),
            (x: 5.0, y: 6.0),
            (x: 7.0, y: 8.0),
        ]];
        assert_eq!(merge_linestring_optimized(input), output);
    }

    #[test]
    fn disjoint_two() {
        let input = vec![
            line_string![( x: 1., y: 1. ), ( x: 2., y: 2. )],
            line_string![( x: 3., y: 3. ), ( x: 4., y: 4. )],
        ];
        let output = input.clone();
        assert_eq!(merge_linestring_optimized(input), output);
    }

    #[test]
    fn disjoin_with_touch() {
        let input = vec![
            line_string![(x: 1., y: 1.), (x: 2., y: 2.)],
            line_string![(x: 2., y: 2.), (x: 3., y: 3.)],
            line_string![(x: 3., y: 3.), (x: 4., y: 4.)],
            line_string![(x: 7., y: 7.), (x: 8., y: 8.)],
        ];
        let output = merge_linestring_optimized(input);
        assert!(output.contains(&line_string![
            (x: 1., y: 1.),
            (x: 2., y: 2.),
            (x: 3., y: 3.),
            (x: 4., y: 4.)
        ]));
        assert!(output.contains(&line_string![(x: 7., y: 7.), (x: 8., y: 8.)]))
    }

    #[test]
    fn intersect_three() {
        let input: Vec<LineString> = vec![
            line_string![( x: 1., y: 1. ), ( x: 2., y: 2. )],
            line_string![( x: 2., y: 1. ), ( x: 2., y: 2. )],
            line_string![( x: 1., y: 2. ), ( x: 2., y: 2. )],
        ];
        let output = merge_linestring_optimized(input.clone());
        println!("{:?}, {:?}", &output, &input);
        assert!(output.contains(&input[0]));
        assert!(output.contains(&input[1]));
        assert!(output.contains(&input[2]));
    }

    #[test]
    fn intersect_and_disjoint() {
        let input: Vec<LineString> = vec![
            line_string![( x: 1., y: 1. ), ( x: 2., y: 2. )], // intersected
            line_string![( x: 1., y: 2. ), ( x: 2., y: 2. )], // intersected
            line_string![( x: 1., y: 3. ), ( x: 2., y: 2. )], // intersected
            line_string![( x: 3., y: 3. ), ( x: 4., y: 4. )], // disjoint
        ];
        let output = merge_linestring_optimized(input.clone());
        assert!(output.contains(&input[0]));
        assert!(output.contains(&input[1]));
        assert!(output.contains(&input[2]));
        assert!(output.contains(&input[3]));
    }
}
