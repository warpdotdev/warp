use criterion::{Criterion, criterion_group, criterion_main, BenchmarkId, PlotConfiguration, AxisScale};
use std::collections::HashMap;

/// Realistic terminal content samples
fn sample_terminal_output() -> String {
    r#"$ npm install typescript
npm notice
npm notice New minor version of npm available! 8.19.1 -> 9.2.0
npm notice To update run: npm install -g npm@9.2.0
npm notice
added 62 packages, and audited 63 packages in 2s
npm notice
npm notice 5 vulnerabilities found
npm notice

$ ls -la
total 48
drwxr-xr-x   9 user  staff   288 Nov 15 10:23 .
drwxr-xr-x  13 user  staff   416 Nov 15 09:45 ..
-rw-r--r--   1 user  staff  1234 Nov 15 10:23 package.json
-rw-r--r--   1 user  staff   567 Nov 15 10:20 README.md
drwxr-xr-x   3 user  staff    96 Nov 15 10:19 src
drwxr-xr-x   2 user  staff    64 Nov 15 10:18 tests
-rw-r--r--   1 user  staff    89 Nov 15 09:50 .gitignore

$ cargo build --release
   Compiling warp v0.1.0
    Finished release [optimized] target(s) in 45.23s

$ ./target/release/warp --version
warp 0.1.0

$ ps aux | grep warp
user             12345   0.5  1.2  12345678  123456   ??  S     10:23AM   0:05.23 /path/to/warp
user             12346   0.0  0.0  12345678    1234 s000  S+    10:24AM   0:00.01 grep warp

$ find . -name "*.rs" -type f
./src/main.rs
./src/lib.rs
./src/terminal/model.rs
./src/terminal/find.rs
./src/terminal/async_find.rs
./tests/find_tests.rs
./tests/async_find_tests.rs

$ docker ps -a
CONTAINER ID   IMAGE             COMMAND              CREATED        STATUS
abc123def456   ubuntu:20.04      "/bin/bash"          2 days ago     Up 2 hours
def456ghi789   postgres:14       "postgres"           1 week ago     Exited (0) 2 days ago
ghi789jkl012   redis:7           "redis-server"       2 weeks ago    Up 5 days

$ git log --oneline | head -20
a1b2c3d Fix async find refinement optimization
e4f5g6h Add query refinement detection
i7j8k9l Implement refinement scan logic
m0n1o2p Improve find performance
q3r4s5t Add unit tests for find
u6v7w8x Initial find implementation
y9z0a1b Add documentation
c2d3e4f Refactor grid handler
g5h6i7j Update dependencies
k8l9m0n Fix memory leak in buffer

$ curl -i https://api.example.com/search?query=warp
HTTP/2 200
content-type: application/json
content-length: 1234
date: Wed, 15 Nov 2023 10:25:00 GMT

{"results": [{"name": "warp", "version": "0.1.0"}]}

$ grep -r "async_find" . --include="*.rs"
./src/terminal/find/model/async_find.rs:pub struct AsyncFindController {
./src/terminal/find/model/async_find.rs:impl AsyncFindController {
./src/terminal/find/model/async_find_tests.rs:fn test_async_find_produces_same_results() {
./tests/async_find_tests.rs:fn test_refinement_scan() {
"#.to_string()
}

/// Create a multi-block terminal with realistic content
fn create_mock_block_list() -> (warp_terminal::model::blocks::BlockList, Vec<String>) {
    use warp_terminal::model::blocks::{BlockList, BlockListItem, TotalIndex};
    use warp_terminal::model::grid::grid_handler::AbsolutePoint;

    let mut block_list = BlockList::new();
    let base_content = sample_terminal_output();

    // Add multiple blocks with variations of the content
    let block_contents = vec![
        base_content.clone(),
        base_content.replace("warp", "WARP").replace("find", "FIND"),
        base_content.clone(),
        "$ searching for async patterns\n$ find . -type f -name '*.rs' | grep async\n"
            .to_string() + &base_content,
        base_content.replace("async_find", "search_async_find"),
    ];

    // Store block contents for reference
    let block_ids: Vec<String> = (0..block_contents.len())
        .map(|i| format!("block_{}", i))
        .collect();

    block_ids.clone()
}

/// Benchmark: Full find operation on realistic content
/// Measures the synchronous find performance (excludes async/UI overhead)
fn bench_full_find_operation(c: &mut Criterion) {
    let mut group = c.benchmark_group("async_find_full_operation");

    // Adjust plotting to show time on log scale for better visibility
    let plot_config = PlotConfiguration::default()
        .summary_scale(AxisScale::Logarithmic);
    group.plot_config(plot_config);

    let queries = vec![
        ("find", "short_common"),
        ("async_find", "medium_specific"),
        ("AsyncFindController", "long_specific"),
        ("grep -r", "with_special_chars"),
    ];

    for (query, description) in queries {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_query", description)),
            &query,
            |b, &query| {
                let content = sample_terminal_output();
                // Repeat content to simulate large terminal
                let large_content = content.repeat(10);

                b.iter(|| {
                    // Simple line-by-line search (simulates core find logic)
                    let matches = large_content
                        .lines()
                        .enumerate()
                        .filter_map(|(line_num, line)| {
                            if line.contains(query) {
                                Some(line_num)
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>();

                    matches.len()
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Query refinement vs full re-scan
/// This is the key optimization: typing another character should be much faster
fn bench_refinement_vs_rescan(c: &mut Criterion) {
    let mut group = c.benchmark_group("async_find_refinement_vs_rescan");

    let plot_config = PlotConfiguration::default()
        .summary_scale(AxisScale::Logarithmic);
    group.plot_config(plot_config);

    let content = sample_terminal_output().repeat(20); // Simulate large terminal history
    let lines: Vec<&str> = content.lines().collect();

    // Scenario: user types "a" then "as" then "asy"
    let refinement_steps = vec![
        ("a", "as", "a_to_as"),
        ("as", "asy", "as_to_asy"),
        ("asy", "asyn", "asy_to_asyn"),
        ("search", "searcht", "search_to_searcht"),
    ];

    for (old_query, new_query, description) in refinement_steps {
        group.bench_with_input(
            BenchmarkId::new("full_rescan", description),
            &(old_query, new_query),
            |b, &(_, new_query)| {
                b.iter(|| {
                    // Full re-scan: search all lines again
                    lines
                        .iter()
                        .filter(|line| line.contains(new_query))
                        .count()
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("filtered_refinement", description),
            &(old_query, new_query),
            |b, &(old_query, new_query)| {
                // First, get matches for old query (this is cached in real scenario)
                let old_matches: Vec<_> = lines
                    .iter()
                    .filter(|line| line.contains(old_query))
                    .collect();

                b.iter(|| {
                    // Refinement: filter existing results
                    old_matches
                        .iter()
                        .filter(|line| line.contains(new_query))
                        .count()
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Case sensitivity impact on refinement
fn bench_case_sensitivity(c: &mut Criterion) {
    let mut group = c.benchmark_group("async_find_case_sensitivity");

    let content = sample_terminal_output().repeat(15);
    let lines: Vec<&str> = content.lines().collect();

    for (query, description) in &[("warp", "warp"), ("WARP", "warp_uppercase")] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_case_sensitive", description)),
            &query,
            |b, &query| {
                b.iter(|| {
                    lines.iter().filter(|l| l.contains(query)).count()
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_case_insensitive", description)),
            &query,
            |b, &query| {
                let lower_query = query.to_lowercase();
                b.iter(|| {
                    lines.iter()
                        .filter(|l| l.to_lowercase().contains(&lower_query))
                        .count()
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    name = benches;
    config = Criterion::default().sample_size(50);
    targets = bench_full_find_operation, bench_refinement_vs_rescan, bench_case_sensitivity
);
criterion_main!(benches);
