# Benchmark: agent-config (Python vs Rust)

Methodology: hyperfine, 3 warmup + 20 runs (help) or 15 runs (cursor no-op), NO_COLOR=1, same minimal repo.
Repo: temp fake repo, HOME isolated.

## Cold start (`--help`)
- Python: 72.8 ms ±0.5 ms, 15.6 MiB
- Rust release: 13.6 ms ±0.2 ms, 6.7 MiB

## No-op workload (`cursor --profile shared`, second run skips)
- Python: 73.1 ms ±1.1 ms, 15.5 MiB
- Rust release: 13.4 ms ±0.3 ms, 6.7 MiB

Rust is ~5.3x faster and ~2.3x lower memory.