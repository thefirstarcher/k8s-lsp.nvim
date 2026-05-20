//! Hot-path microbenchmarks: schema lookup, completion candidate generation,
//! hover render. These exist so regressions show up as a number, not a vibe.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use k8s_lsp_schema::{fields_at, render_hover, schema_at_path, PathSeg, SchemaRegistry};

fn bench_schema_lookup(c: &mut Criterion) {
    let registry = SchemaRegistry::new();
    c.bench_function("schema_lookup_deployment", |b| {
        b.iter(|| {
            let s = registry.lookup(black_box("apps/v1"), black_box("Deployment"));
            black_box(s);
        });
    });
}

fn bench_fields_at(c: &mut Criterion) {
    let registry = SchemaRegistry::new();
    let schema = registry
        .lookup("apps/v1", "Deployment")
        .expect("Deployment schema present in embedded bundle");
    let path = vec![PathSeg::Key("spec".into())];
    c.bench_function("fields_at_deployment_spec", |b| {
        b.iter(|| {
            let v = fields_at(black_box(&schema), black_box(&path));
            black_box(v);
        });
    });
}

fn bench_hover_render(c: &mut Criterion) {
    let registry = SchemaRegistry::new();
    let schema = registry
        .lookup("apps/v1", "Deployment")
        .expect("Deployment schema present in embedded bundle");
    let path = vec![PathSeg::Key("spec".into()), PathSeg::Key("replicas".into())];
    let node = schema_at_path(&schema, &path).expect("spec.replicas exists");
    c.bench_function("render_hover_replicas", |b| {
        b.iter(|| {
            let md = render_hover(black_box("Deployment.spec.replicas"), black_box(node));
            black_box(md);
        });
    });
}

criterion_group!(benches, bench_schema_lookup, bench_fields_at, bench_hover_render);
criterion_main!(benches);
