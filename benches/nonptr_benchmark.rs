use criterion::BenchmarkId;
use criterion::Criterion;
use criterion::{criterion_group, criterion_main};
use tokio::runtime::Builder;

use nonlocking::nonpointermap::NonPointerMap;
use std::sync::Arc;
// This is a struct that tells Criterion.rs to use the "futures" crate's current-thread executor
#[cfg(feature = "async_tokio")]
use criterion::async_executor::AsyncExecutor;

// Here we have an async function to benchmark
async fn do_something(size: usize, keys: Arc<Vec<Vec<(String, String)>>>) {
    let map: NonPointerMap<String, String> = NonPointerMap::new();

    let mut handles = Vec::new();

    for thread in 0..size {
        let map = map.clone();
        let keys = keys.clone();

        handles.push(tokio::task::spawn(async move {
            for step in 0..1000 {
                let step_pair: &(String, String) = &(keys[(step / 100) % 10][(step + thread) % 10]);

                let future = map.get(
                    &step_pair.0,
                    Box::new(move |key: &String| format!("Valu: {}", key)),
                );

                let value = future.await;

                assert_eq!(*step_pair.1, value);
            }
        }));
    }

    for handle in handles {
        handle.await;
    }
}

fn from_elem(c: &mut Criterion) {
    let size: usize = 1024;

    let keys = vec![
        "one".to_owned(),
        "two".to_owned(),
        "three".to_owned(),
        "four".to_owned(),
        "five".to_owned(),
        "six".to_owned(),
        "seven".to_owned(),
        "eight".to_owned(),
        "nine".to_owned(),
        "ten".to_owned(),
    ];

    let mut keysVec: Vec<Vec<(String, String)>> = Vec::new();

    for i in 0..10 {
        keysVec.push(
            keys.iter()
                .map(|text| text.clone() + &i.to_string())
                .map(|key| {
                    let value = format!("Valu: {}", key);
                    (key, value)
                })
                .collect(),
        );
    }

    let keys = Arc::new(keysVec);

    c.bench_with_input(BenchmarkId::new("nonptr", size), &size, move |b, &s| {
        let runtime = Builder::new_multi_thread()
            .worker_threads(3)
            .thread_name("benchmark")
            .thread_stack_size(3 * 1024 * 1024)
            .build()
            .unwrap();
        // Insert a call to `to_async` to convert the bencher to async mode.
        // The timing loops are the same as with the normal bencher.
        b.to_async(runtime).iter(|| do_something(s, keys.clone()));
    });
}

criterion_group!(benches, from_elem);
criterion_main!(benches);
