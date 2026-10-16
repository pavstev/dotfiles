use std::process::Command;
use std::time::Instant;
fn main() {
    let bin = env!("CARGO_BIN_EXE_agent-config");
    let t = Instant::now();
    let mut n = 0;
    while t.elapsed().as_secs() < 2 {
        let _ = Command::new(bin).arg("--help").output();
        n += 1;
    }
    println!("rust invocations in 2s: {} ({:.2} ms each)", n, t.elapsed().as_secs_f64()*1000.0/n as f64);
}