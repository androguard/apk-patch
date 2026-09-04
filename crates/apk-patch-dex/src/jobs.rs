//! Parallel job pool helper for `-j` / `--jobs`.

#[cfg(feature = "parallel")]
pub fn with_jobs<R: Send>(jobs: usize, f: impl FnOnce() -> R + Send) -> R {
    let threads = jobs.max(1);
    if threads == 1 {
        return f();
    }
    match rayon::ThreadPoolBuilder::new().num_threads(threads).build() {
        Ok(pool) => pool.install(f),
        Err(_) => f(),
    }
}

#[cfg(not(feature = "parallel"))]
pub fn with_jobs<R>(_jobs: usize, f: impl FnOnce() -> R) -> R {
    f()
}
