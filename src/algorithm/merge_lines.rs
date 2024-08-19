use geo::{Contains, Coord, CoordsIter, GeoFloat, Intersects, LineString};
use itertools::Itertools;
use rayon::iter::{ParallelBridge, ParallelIterator};
use rstar::RTreeObject;

// Used to merge two linestrings that intersect on either endpoint.
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

// Changes the startpoint/endpoint of a closed linestring.
fn rotate_start_point<T: GeoFloat>(linestring: &LineString<T>, at: Coord<T>) -> LineString<T> {
    let coords = linestring.coords_iter();
    let count = coords.len();
    let mut repeated = std::iter::repeat(coords).flatten();
    loop {
        if let Some(coord) = repeated.next() {
            // println!("{:?}, {:?}", coord, at);
            if coord.intersects(&at) {
                return LineString::from_iter(std::iter::once(coord).chain(repeated.take(count)));
            }
        }
    }
}

// Get the lines in 'linestrings' that intersect 'linestring'.
// Returns a vector of references to those lines and their index
// in the vector.
fn intersected_lines<'a, T: GeoFloat + Send + Sync>(
    linestrings: &'a Vec<Option<LineString<T>>>,
    linestring: &LineString<T>,
) -> (Vec<usize>, Vec<&'a LineString<T>>) {
    linestrings
        .iter()
        .enumerate()
        .par_bridge()
        .filter_map(|(index, other_linestring)| {
            if let Some(other_linestring) = other_linestring {
                if other_linestring.intersects(linestring)
                    && !std::ptr::addr_eq(other_linestring, linestring)
                {
                    return Some((index, other_linestring));
                }
            }
            None
        })
        .unzip()
}

// Merges a single linestring.
fn compute_linestring<T: GeoFloat + Send + Sync>(
    linestrings: &Vec<Option<LineString<T>>>,
    linestring: &LineString<T>,
) -> (Option<LineString<T>>, Vec<usize>) {
    let intersected = intersected_lines(linestrings, linestring);
    // The number of endpoints that intersect the start point of 'linestring'.
    let start_point_count: usize = intersected
        .1
        .iter()
        .map(|other| {
            if other.intersects(&linestring.0[0]) {
                return 1;
            }
            0
        })
        .sum();
    // The number of endpoints that intersect the end point of 'linestring'.
    let end_point_count: usize = intersected
        .1
        .iter()
        .map(|other| {
            if other.intersects(&linestring.0[linestring.coords_count() - 1]) {
                return 1;
            }
            0
        })
        .sum();
    // The computed linestring.
    let mut result = None;
    // A vector of indices that should be set to None in the 'linestrings' vector.
    let mut to_remove = Vec::new();
    // Loop over the intersected lines and merge them with 'linestring' if they are the only
    // one that intersect the 'linestring' in a particular 'linestring' endpoint.
    for (index, other) in std::iter::zip(intersected.0.into_iter(), intersected.1.iter()) {
        if (start_point_count == 1 && other.intersects(&linestring.0[0]))
            || (end_point_count == 1
                && other.intersects(&linestring.0[linestring.coords_count() - 1]))
        {
            if let None = result {
                if let Some(merged) = merge_two(linestring, other) {
                    result = Some(merged);
                    to_remove.push(index);
                } else {
                    result = Some(linestring.clone())
                }
            } else {
                if let Some(merged) = merge_two(result.as_ref().unwrap(), other) {
                    result = Some(merged);
                    to_remove.push(index);
                }
            }
            if let Some(ref mut result) = result {
                // Handles special case where result is closed.
                // Without this block, the starpoint/endpoint of the closed linestring
                // wont be in the same place as the endpoint of another
                // linestring that intersects this closed linestring.
                if result.is_closed() {
                    let coord = linestrings.iter().find_map(|linestring| {
                        if let Some(linestring) = linestring {
                            if result.contains(linestring) || std::ptr::addr_eq(linestring, other) {
                                return None;
                            } else {
                                for coord in result.coords_iter() {
                                    if linestring.0[0].intersects(&coord) {
                                        return Some(linestring.0[0]);
                                    }
                                    if linestring.0[linestring.coords_count() - 1]
                                        .intersects(&coord)
                                    {
                                        return Some(linestring.0[linestring.coords_count() - 1]);
                                    }
                                }
                            }
                        }
                        None
                    });
                    if let Some(coord) = coord {
                        *result = rotate_start_point(result, coord);
                    }
                }
            }
        }
    }
    // Return the merged linestring and the indices that should be set to None.
    (result, to_remove)
}

fn _dedup_linestrings<T: GeoFloat + Send + Sync>(
    lines: Vec<LineString<T>>,
) -> Vec<geo::LineString<T>> {
    let mut lines_dedup = Vec::new();
    for line in lines {
        if !lines_dedup.contains(&line) {
            lines_dedup.push(line);
        }
    }
    lines_dedup
}

pub fn merge_linestrings<T: GeoFloat + Send + Sync>(
    lines: Vec<LineString<T>>,
) -> Vec<LineString<T>> {
    // Is it okay to dedup the linestrings in a tool like this?
    // let lines = _dedup_linestrings(lines);

    let mut linestrings: Vec<Option<LineString<T>>> =
        lines.into_iter().map(|line| Some(line.into())).collect();
    let mut prev_count = 0;
    let mut iter = 1;
    loop {
        for i in 0..linestrings.len() {
            let linestring = &mut linestrings[i];
            if linestring.is_some() {
                let linestring_ref = linestring.take().unwrap();
                let computed = compute_linestring(&linestrings, &linestring_ref);
                if let Some(computed_linestring) = computed.0 {
                    linestrings.get_mut(i).unwrap().replace(computed_linestring);
                    for index in computed.1.into_iter() {
                        linestrings.get_mut(index).unwrap().take();
                    }
                } else {
                    linestrings.get_mut(i).unwrap().replace(linestring_ref);
                }
            }
        }
        let count = linestrings.iter().filter(|line| line.is_some()).count();
        println!(
            "Iteration {} completed. Prev. count: {}, Current count: {}",
            iter, prev_count, count
        );
        if prev_count == count {
            // Stop when it converges.
            return linestrings
                .into_iter()
                .filter_map(|linestring| linestring)
                .collect();
        } else {
            prev_count = count;
        }
        iter += 1;
    }
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
        assert_eq!(merge_linestrings(input), output);
    }

    #[test]
    fn touches_two() {
        let input = vec![
            line_string![(x: 1., y: 1.), (x: 2., y: 2.)],
            line_string![(x: 2., y: 2.), (x: 3., y: 3.)],
        ];
        let output = merge_linestrings(input.clone());
        assert!(output.contains(&line_string![
            (x: 1., y: 1.),
            (x: 2., y: 2.),
            (x: 3., y: 3.)
        ]));
    }

    #[test]
    fn touches_three() {
        let input = vec![
            line_string![(x: -21.95156, y: 64.1446), (x: -21.951, y: 64.14479)],
            line_string![(x: -21.951, y: 64.14479), (x: -21.95044, y: 64.14527)],
            line_string![(x: -21.95044, y: 64.14527), (x: -21.951445, y: 64.145508)],
        ];
        let output = vec![line_string![
            (x: -21.95156, y: 64.1446),
            (x: -21.951, y: 64.14479),
            (x: -21.95044, y: 64.14527),
            (x: -21.951445, y: 64.145508),
        ]];
        assert_eq!(merge_linestrings(input), output);
    }

    #[test]
    fn disjoint_two() {
        let input = vec![
            line_string![( x: 1., y: 1. ), ( x: 2., y: 2. )],
            line_string![( x: 3., y: 3. ), ( x: 4., y: 4. )],
        ];
        let output = input.clone();
        assert_eq!(merge_linestrings(input), output);
    }

    #[test]
    fn disjoin_with_touch() {
        let input = vec![
            line_string![(x: 1., y: 1.), (x: 2., y: 2.)],
            line_string![(x: 2., y: 2.), (x: 3., y: 3.)],
            line_string![(x: 3., y: 3.), (x: 4., y: 4.)],
            line_string![(x: 7., y: 7.), (x: 8., y: 8.)],
        ];
        let output = merge_linestrings(input);
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
        let output = merge_linestrings(input.clone());
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
        let output = merge_linestrings(input.clone());
        assert!(output.contains(&input[0]));
        assert!(output.contains(&input[1]));
        assert!(output.contains(&input[2]));
        assert!(output.contains(&input[3]));
    }

    // #[test]
    // fn test_big_dataset() {
    //     let dataset = VectorDataset::new("./assets/lines_smaller.shp");
    //     let lines = flatten_linestrings(dataset.to_geo().unwrap());
    //     let computed = merge_linestrings(lines);
    //     assert!(computed.len() != 0);
    //     geometries_to_file(computed, "./assets/lines_smaller_merged.shp", None, None);
    // }
}
