use collie_search::benchmark::{build_benchmark_setup, command_available};
use criterion::{Criterion, criterion_group, criterion_main};
use std::process::Command;
use tempfile::TempDir;

fn search_benchmarks(c: &mut Criterion) {
    let temp = TempDir::new().expect("tempdir");
    let setup = build_benchmark_setup(temp.path()).expect("benchmark setup");
    let corpus_path = setup.corpus_path.clone();
    let builder = setup.builder;

    let mut group = c.benchmark_group("search");
    group.bench_function("collie_exact", |b| {
        b.iter(|| builder.search_pattern("handle_request"));
    });
    group.bench_function("collie_prefix", |b| {
        b.iter(|| builder.search_pattern("handle%"));
    });
    group.bench_function("collie_substring", |b| {
        b.iter(|| builder.search_pattern("%request%"));
    });

    if command_available("grep") {
        group.bench_function("grep_exact", |b| {
            b.iter(|| {
                Command::new("grep")
                    .args(["-rl", "handle_request"])
                    .arg(&corpus_path)
                    .output()
                    .expect("grep exact");
            });
        });
        group.bench_function("grep_substring", |b| {
            b.iter(|| {
                Command::new("grep")
                    .args(["-rl", "request"])
                    .arg(&corpus_path)
                    .output()
                    .expect("grep substring");
            });
        });
    }

    if command_available("rg") {
        group.bench_function("rg_exact", |b| {
            b.iter(|| {
                Command::new("rg")
                    .args(["-l", "handle_request"])
                    .arg(&corpus_path)
                    .output()
                    .expect("rg exact");
            });
        });
        group.bench_function("rg_substring", |b| {
            b.iter(|| {
                Command::new("rg")
                    .args(["-l", "request"])
                    .arg(&corpus_path)
                    .output()
                    .expect("rg substring");
            });
        });
    }

    group.finish();
}

criterion_group!(benches, search_benchmarks);
criterion_main!(benches);
