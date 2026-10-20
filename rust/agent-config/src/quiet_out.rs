//! `println!` and `print!` that stop quietly on a closed pipe (`mic_check | head`), like the shell
//! scripts did, instead of panicking with "failed printing to stdout". Included by the bins with
//! `#[macro_use] #[path = "../quiet_out.rs"] mod quiet_out;`; the macros shadow the std ones.
//! Any other write error is treated the same way: nothing can be printed, so exit 0 at once.

#![allow(unused_macros)] // a bin may use only one of the two

use std::io::Write;

pub fn emit(args: std::fmt::Arguments) {
    let mut out = std::io::stdout().lock();
    if out.write_fmt(args).and_then(|()| out.flush()).is_err() {
        std::process::exit(0);
    }
}

macro_rules! println {
    () => { $crate::quiet_out::emit(format_args!("\n")) };
    ($($arg:tt)*) => { $crate::quiet_out::emit(format_args!("{}\n", format_args!($($arg)*))) };
}

macro_rules! print {
    ($($arg:tt)*) => { $crate::quiet_out::emit(format_args!($($arg)*)) };
}
