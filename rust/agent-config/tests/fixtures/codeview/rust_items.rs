use std::fmt::{self, Display};

pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    pub fn raw(&self) -> &'static str {
        r#"{ "brace": "}" } and a quote " inside"#
    }

    pub fn hashes(&self) -> &'static str {
        r##"has "# inside and }"##
    }
}

impl std::fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

struct Pair<T>(T, T);

impl<T: Copy> Pair<T> {
    fn first(&self) -> T { self.0 }
}

macro_rules! square {
    ($x:expr) => {
        $x * $x
    };
}

macro_rules! open_brace {
    () => {
        "{"
    };
}

mod inner {
    pub fn deep() {}
}
