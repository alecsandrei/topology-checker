mod support;
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use std::{hint::black_box, time::Duration};

fn benchmarks(c: &mut Criterion) {
    let input = support::multipart();
    c.bench_function("patterns/multipart", |b| {
        b.iter_batched(
            || input.clone(),
            |v| black_box(support::clean(v)),
            BatchSize::LargeInput,
        )
    });
    let input = support::aggregation();
    c.bench_function("patterns/aggregation", |b| {
        b.iter_batched(
            || input.clone(),
            |v| black_box(support::aggregate(v)),
            BatchSize::LargeInput,
        )
    });
    let input = support::long_lines();
    c.bench_function("patterns/long-lines", |b| {
        b.iter_batched(
            || input.clone(),
            |v| black_box(support::merge(v)),
            BatchSize::LargeInput,
        )
    });
}
criterion_group! { name = benches; config = Criterion::default().sample_size(30).warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(3)); targets = benchmarks }
criterion_main!(benches);
