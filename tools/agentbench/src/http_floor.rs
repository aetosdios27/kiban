use crate::{
    Config,
    backend::{Helix, Result},
    metrics::Histogram,
    runner::{emit, row},
};
use serde_json::json;
use std::{io::Write, time::Instant};

pub fn floor(b: &Helix, c: &Config, out: &mut impl Write) -> Result<()> {
    b.noop()?; // Establish this client’s persistent connection outside timing.
    let started = Instant::now();
    let mut h = Histogram::default();
    let mut call_ns = 0;
    for _ in 0..c.operations {
        let start = Instant::now();
        b.noop()?;
        let elapsed = start.elapsed().as_nanos();
        call_ns += elapsed;
        h.record(u64::try_from(elapsed)?);
    }
    emit(
        out,
        &row(
            "transport-floor",
            &h,
            &h,
            &Histogram::default(),
            started,
            call_ns,
            0,
        ),
    )?;
    emit(
        out,
        &json!({"type": "complete", "verified_responses": c.operations}),
    )
}
