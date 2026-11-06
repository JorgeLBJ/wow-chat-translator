//! Translation worker pool: runs pipeline jobs with the current translator (HTTP or the
//! Claude Code CLI, so seconds at worst) and hands each result back to the pipeline.

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use crate::core::pipeline::Job;

use super::shared::Shared;

pub const WORKERS: usize = 2;

pub fn spawn(shared: Arc<Shared>, jobs: Receiver<Job>) {
    let jobs = Arc::new(Mutex::new(jobs));
    for n in 0..WORKERS {
        let shared = shared.clone();
        let jobs = jobs.clone();
        let _ = std::thread::Builder::new().name(format!("translate-{n}")).spawn(move || loop {
            // Hold the receiver lock only while waiting, never while translating.
            let job = match jobs.lock().unwrap_or_else(|e| e.into_inner()).recv() {
                Ok(job) => job,
                Err(_) => return, // app is closing
            };
            let translator = shared.translator();
            let result = translator.translate(&job.text, &job.target);
            if let Err(e) = &result {
                shared.log.write(&format!("translate error id={} {e}", job.id));
            }
            shared.complete(job.id, result);
        });
    }
}
