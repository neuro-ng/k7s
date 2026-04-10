//! Benchmarks for the sanitizer pipeline — Phase 14.1.
//!
//! Targets from CLAUDE.md:
//!   - Log processing: 50K lines/sec
//!   - AI context send: sanitized context within 100ms of user query
//!
//! Run with: `cargo bench`

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde_json::json;

use k7s::client::Gvr;
use k7s::config::SanitizerConfig;
use k7s::sanitizer::{compress, sanitize};

// ─── Sanitizer pipeline ───────────────────────────────────────────────────────

fn bench_sanitize_pod(c: &mut Criterion) {
    let gvr = Gvr::core("v1", "pods");
    let cfg = SanitizerConfig::default();

    let pod = json!({
        "metadata": {
            "name": "nginx-abc123",
            "namespace": "default",
            "labels": {
                "app": "nginx",
                "version": "1.25.0"
            },
            "annotations": {
                "kubectl.kubernetes.io/last-applied-configuration": "..."
            },
            "creationTimestamp": "2024-01-15T10:00:00Z"
        },
        "spec": {
            "containers": [{
                "name": "nginx",
                "image": "nginx:1.25.0",
                "resources": {
                    "requests": { "cpu": "100m", "memory": "128Mi" },
                    "limits": { "cpu": "500m", "memory": "512Mi" }
                },
                "env": [
                    { "name": "PORT", "value": "8080" },
                    { "name": "DB_PASSWORD", "value": "supersecret" },
                    { "name": "API_KEY", "value": "sk-abc123xyz" }
                ]
            }]
        },
        "status": {
            "phase": "Running",
            "conditions": [
                { "type": "Ready", "status": "True" }
            ],
            "containerStatuses": [{
                "name": "nginx",
                "ready": true,
                "restartCount": 0
            }]
        }
    });

    c.bench_function("sanitize_pod", |b| {
        b.iter(|| {
            sanitize(
                black_box(&gvr),
                black_box(Some("default")),
                black_box("nginx-abc123"),
                black_box(pod.clone()),
                black_box(&cfg),
            )
            .unwrap()
        })
    });
}

fn bench_sanitize_secret(c: &mut Criterion) {
    let gvr = Gvr::core("v1", "secrets");
    let cfg = SanitizerConfig::default();

    let secret = json!({
        "metadata": {
            "name": "my-secret",
            "namespace": "default"
        },
        "type": "Opaque",
        "data": {
            "password": "c3VwZXJzZWNyZXQ=",
            "api-key": "c2stYWJjMTIzeHl6"
        }
    });

    c.bench_function("sanitize_secret_strips_data", |b| {
        b.iter(|| {
            sanitize(
                black_box(&gvr),
                black_box(Some("default")),
                black_box("my-secret"),
                black_box(secret.clone()),
                black_box(&cfg),
            )
            .unwrap()
        })
    });
}

// ─── Log compression ──────────────────────────────────────────────────────────

fn bench_log_compress(c: &mut Criterion) {
    let mut group = c.benchmark_group("log_compress");

    for &line_count in &[1_000usize, 10_000, 50_000] {
        // Realistic log: ~80% repeated errors, ~20% unique info lines.
        let mut lines: Vec<String> = Vec::with_capacity(line_count);
        let error_count = (line_count as f64 * 0.8) as usize;
        let info_count = line_count - error_count;

        for _ in 0..error_count {
            lines.push("ERROR: connection refused to 10.0.0.1:5432 after 3 retries".to_owned());
        }
        for i in 0..info_count {
            lines.push(format!("INFO: processed request {i} in 12ms"));
        }

        group.throughput(Throughput::Elements(line_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(line_count),
            &lines,
            |b, lines| {
                b.iter(|| compress(black_box(lines), black_box(500)));
            },
        );
    }
    group.finish();
}

// ─── Token estimation ─────────────────────────────────────────────────────────

fn bench_token_estimate(c: &mut Criterion) {
    // Simulate a typical cluster context block (~2000 chars).
    let text = "k7s AI context: pods/nginx-abc123 Running 2/2 restarts=0 age=5d\n".repeat(30);

    c.bench_function("estimate_tokens_2k_chars", |b| {
        b.iter(|| k7s::ai::token_budget::estimate_tokens(black_box(&text)))
    });
}

// ─── Groups ───────────────────────────────────────────────────────────────────

criterion_group!(sanitizer_benches, bench_sanitize_pod, bench_sanitize_secret,);

criterion_group!(log_benches, bench_log_compress,);

criterion_group!(ai_benches, bench_token_estimate,);

criterion_main!(sanitizer_benches, log_benches, ai_benches);
