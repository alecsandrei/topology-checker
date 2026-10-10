use topology_checker_core::merge_lines;

mod original {

    use geo::{LineString, line_string};
    use topology_checker_core::merge_lines;

    #[test]
    fn test_one() {
        let input = vec![line_string![
            (x: 1., y: 1.),
            (x: 2., y: 2.),
        ]];
        let output = input.clone();
        assert_eq!(merge_lines(input, 0.).unwrap(), output);
    }

    #[test]
    fn touches_two() {
        let input = vec![
            line_string![(x: 1., y: 1.), (x: 2., y: 2.)],
            line_string![(x: 2., y: 2.), (x: 3., y: 3.)],
        ];
        let output = merge_lines(input.clone(), 0.).unwrap();
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
        assert_eq!(merge_lines(input, 0.).unwrap(), output);
    }

    #[test]
    fn disjoint_two() {
        let input = vec![
            line_string![( x: 1., y: 1. ), ( x: 2., y: 2. )],
            line_string![( x: 3., y: 3. ), ( x: 4., y: 4. )],
        ];
        let output = input.clone();
        assert_eq!(merge_lines(input, 0.).unwrap(), output);
    }

    #[test]
    fn disjoin_with_touch() {
        let input = vec![
            line_string![(x: 1., y: 1.), (x: 2., y: 2.)],
            line_string![(x: 2., y: 2.), (x: 3., y: 3.)],
            line_string![(x: 3., y: 3.), (x: 4., y: 4.)],
            line_string![(x: 7., y: 7.), (x: 8., y: 8.)],
        ];
        let output = merge_lines(input, 0.).unwrap();
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
        let output = merge_lines(input.clone(), 0.).unwrap();
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
        let output = merge_lines(input.clone(), 0.).unwrap();
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
mod optimized_regressions {

    use geo::{LineString, line_string};
    use topology_checker_core::merge_lines;

    #[test]
    fn test_one() {
        let input = vec![line_string![
            (x: 1., y: 1.),
            (x: 2., y: 2.),
        ]];
        let output = input.clone();
        assert_eq!(merge_lines(input, 0.).unwrap(), output);
    }

    #[test]
    fn touches_two() {
        let input = vec![
            line_string![(x: 1., y: 1.), (x: 2., y: 2.)],
            line_string![(x: 2., y: 2.), (x: 3., y: 3.)],
        ];
        let output = merge_lines(input.clone(), 0.).unwrap();
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
        assert_eq!(merge_lines(input, 0.).unwrap(), output);
    }

    #[test]
    fn disjoint_two() {
        let input = vec![
            line_string![( x: 1., y: 1. ), ( x: 2., y: 2. )],
            line_string![( x: 3., y: 3. ), ( x: 4., y: 4. )],
        ];
        let output = input.clone();
        assert_eq!(merge_lines(input, 0.).unwrap(), output);
    }

    #[test]
    fn disjoin_with_touch() {
        let input = vec![
            line_string![(x: 1., y: 1.), (x: 2., y: 2.)],
            line_string![(x: 2., y: 2.), (x: 3., y: 3.)],
            line_string![(x: 3., y: 3.), (x: 4., y: 4.)],
            line_string![(x: 7., y: 7.), (x: 8., y: 8.)],
        ];
        let output = merge_lines(input, 0.).unwrap();
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
        let output = merge_lines(input.clone(), 0.).unwrap();
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
        let output = merge_lines(input.clone(), 0.).unwrap();
        assert!(output.contains(&input[0]));
        assert!(output.contains(&input[1]));
        assert!(output.contains(&input[2]));
        assert!(output.contains(&input[3]));
    }
}

mod additional {
    use geo::{LineString, line_string};
    use topology_checker_core::{explode_lines, merge_lines};
    #[test]
    fn loops_and_interior_junctions_preserve_original_parts() {
        let closed = line_string![(x:0.,y:0.),(x:1.,y:0.),(x:1.,y:1.),(x:0.,y:0.)];
        assert_eq!(merge_lines([closed.clone()], 0.).unwrap(), vec![closed]);
        let a = line_string![(x:0.,y:0.),(x:2.,y:0.)];
        let b = line_string![(x:1.,y:-1.),(x:1.,y:0.)];
        let c = line_string![(x:1.,y:0.),(x:1.,y:1.)];
        assert_eq!(
            merge_lines([a.clone(), b.clone(), c.clone()], 0.).unwrap(),
            vec![a, b, c]
        );
    }
    #[test]
    fn isolated_cycle_and_tolerance_join() {
        let lines = [
            line_string![(x:0.,y:0.),(x:1.,y:0.)],
            line_string![(x:1.,y:0.),(x:1.,y:1.)],
            line_string![(x:1.,y:1.),(x:0.,y:0.)],
        ];
        let merged = merge_lines(lines, 0.).unwrap();
        assert_eq!(merged.len(), 1);
        assert!(merged[0].is_closed());
        assert_eq!(
            merge_lines(
                [
                    line_string![(x:0.,y:0.),(x:1.,y:0.)],
                    line_string![(x:1.000000001,y:0.),(x:2.,y:0.)]
                ],
                0.001
            )
            .unwrap()
            .len(),
            1
        );
    }
    #[test]
    fn duplicate_segments_are_preserved_and_empty_parts_are_skipped() {
        let a = line_string![(x:0.,y:0.),(x:1.,y:0.)];
        let merged = merge_lines([a.clone(), a.clone(), LineString::empty()], 0.).unwrap();
        assert_eq!(merged.iter().map(|l| l.lines().count()).sum::<usize>(), 2);
        assert_eq!(explode_lines([a.clone(), a]).unwrap().len(), 2);
    }
}

#[test]
fn diagonal_endpoint_tolerance_and_direction_tie() {
    use geo::line_string;
    let a = line_string![(x:0.,y:0.),(x:1.,y:1.)];
    let b = line_string![(x:2.,y:2.),(x:1.000001,y:1.000001)];
    let result = merge_lines([a, b], 0.001).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0[0], geo::coord! {x:0.,y:0.});
    assert_eq!(result[0].0.last().unwrap(), &geo::coord! {x:2.,y:2.});
}
#[test]
fn snapped_zero_length_parts_do_not_create_degenerate_output() {
    use geo::line_string;
    assert!(
        merge_lines([line_string![(x:0.,y:0.),(x:0.00001,y:0.)]], 0.001)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn reversed_long_parts_keep_vertices_snap_join_and_resolve_direction_tie() {
    use geo::{Coord, LineString};
    let first = LineString::from((0..=2_000).map(|i| (i as f64, 0.)).collect::<Vec<_>>());
    let mut second = LineString::from(
        (2_000..=4_000)
            .rev()
            .map(|i| (i as f64, 0.))
            .collect::<Vec<_>>(),
    );
    second.0.last_mut().unwrap().x += 0.0001;
    let merged = merge_lines([first, second], 0.001).unwrap();
    assert_eq!(
        merged,
        vec![LineString(
            (0..=4_000).map(|i| Coord { x: i as f64, y: 0. }).collect()
        )]
    );
}
