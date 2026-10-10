use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use geo::{Geometry, LineString, Point, Rect, coord};
use std::hint::black_box;
use topology_checker_core::{Layer, Rule, Topology, merge_lines};

fn lines(n: usize) -> Vec<Geometry> {
    (0..n)
        .map(|i| Geometry::LineString(LineString::from(vec![(i as f64, 0.), (i as f64 + 1., 0.)])))
        .collect()
}
fn polygons(n: usize) -> Vec<Geometry> {
    (0..n)
        .map(|i| {
            Geometry::Polygon(
                Rect::new(
                    coord! { x: i as f64 * 1.5, y: 0. },
                    coord! { x: i as f64 * 1.5 + 2., y: 2. },
                )
                .to_polygon(),
            )
        })
        .collect()
}
fn run(input: Vec<Geometry>, rule: Rule, n: usize) {
    let mut topology = Topology::new(0.).unwrap();
    let layer = topology.add_layer(Layer::from_geometries("input", input));
    let other = (rule == Rule::PointMustBeInside).then(|| {
        topology.add_layer(Layer::from_geometries(
            "area",
            [Geometry::Polygon(
                Rect::new(
                    coord! { x: -1., y: -1. },
                    coord! { x: n as f64 + 1., y: 1. },
                )
                .to_polygon(),
            )],
        ))
    });
    topology.add_rule(rule, layer, other).unwrap();
    black_box(topology.validate().unwrap());
}
fn benchmarks(c: &mut Criterion) {
    for (name, rule) in [
        ("dangles", Rule::LineMustNotHaveDangles),
        ("polygon-overlaps", Rule::PolygonMustNotOverlap),
        ("point-inside", Rule::PointMustBeInside),
    ] {
        let mut group = c.benchmark_group(name);
        group.sample_size(10);
        for n in [40_000, 80_000, 160_000] {
            let input = match rule {
                Rule::LineMustNotHaveDangles => lines(n),
                Rule::PolygonMustNotOverlap => polygons(n),
                _ => (0..n)
                    .map(|i| Geometry::Point(Point::new(i as f64, 0.)))
                    .collect(),
            };
            group.throughput(Throughput::Elements(n as u64));
            group.bench_with_input(BenchmarkId::from_parameter(n), &input, |b, input| {
                b.iter_batched(
                    || input.clone(),
                    |input| run(input, rule, n),
                    BatchSize::LargeInput,
                )
            });
        }
        group.finish();
    }
    let mut group = c.benchmark_group("merge-lines");
    group.sample_size(10);
    for n in [40_000, 80_000, 160_000] {
        let input = lines(n)
            .into_iter()
            .map(|g| match g {
                Geometry::LineString(l) => l,
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &input, |b, input| {
            b.iter_batched(
                || input.clone(),
                |lines| black_box(merge_lines(lines, 0.).unwrap()),
                BatchSize::LargeInput,
            )
        });
    }
    group.finish();
}
criterion_group! { name = benches; config = Criterion::default().warm_up_time(std::time::Duration::from_secs(1)).measurement_time(std::time::Duration::from_secs(3)); targets = benchmarks }
criterion_main!(benches);
