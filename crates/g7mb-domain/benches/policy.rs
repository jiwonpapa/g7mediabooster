//! Microbenchmarks for pure domain policy hot paths.

use criterion::{Criterion, criterion_group, criterion_main};
use g7mb_domain::{ImageLimits, ImageProbe, ObjectKey};

fn bench_policy(criterion: &mut Criterion) {
    criterion.bench_function("object_key_validation", |bencher| {
        bencher.iter(|| ObjectKey::new("raw/tenant/2026/07/uuid/source.jpg"));
    });
    criterion.bench_function("image_limit_validation", |bencher| {
        let limits = ImageLimits::default();
        let probe = ImageProbe {
            byte_len: 8 * 1024 * 1024,
            width: 4_032,
            height: 3_024,
            frames: 1,
        };
        bencher.iter(|| limits.validate(probe));
    });
}

criterion_group!(benches, bench_policy);
criterion_main!(benches);
