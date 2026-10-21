//! `println!` and `print!` that stop quietly on a closed pipe (`mic_check | head`), like the shell
//! scripts did, instead of panicking with "failed printing to stdout". A bin that wants them
//! imports them over the std ones:
//! `use agent_config::{quiet_print as print, quiet_println as println};`
//! Any other write error is treated the same way: nothing can be printed, so exit 0 at once.

use std::io::Write;

/// Prints to stdout and flushes. On a write error (a closed pipe) it exits 0 at once.
pub fn emit(args: std::fmt::Arguments) {
    let mut out = std::io::stdout().lock();
    if out.write_fmt(args).and_then(|()| out.flush()).is_err() {
        std::process::exit(0);
    }
}

#[macro_export]
macro_rules! quiet_println {
    () => { $crate::quiet_out::emit(format_args!("\n")) };
    ($($arg:tt)*) => { $crate::quiet_out::emit(format_args!("{}\n", format_args!($($arg)*))) };
}

#[macro_export]
macro_rules! quiet_print {
    ($($arg:tt)*) => { $crate::quiet_out::emit(format_args!($($arg)*)) };
}
