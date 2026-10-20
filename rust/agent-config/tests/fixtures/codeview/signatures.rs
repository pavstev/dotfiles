use std::fmt::Debug;

pub fn largest<T>(items: &[T]) -> Option<&T>
where
    T: PartialOrd + Debug,
{
    let mut it = items.iter();
    let mut best = it.next()?;
    for x in it {
        if x > best {
            best = x;
        }
    }
    Some(best)
}

pub async fn connect<'a, S>(
    host: &'a str,
    stream: S,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: Send + 'static,
{
    let _ = (host, stream);
    Ok(())
}

pub trait Shape {
    fn area(&self) -> f64;
    fn name(&self) -> String {
        String::from("shape")
    }
}

fn braces() -> char {
    let open = '{';
    let quote = '\'';
    if open == '{' { quote } else { '}' }
}

pub struct Wrapper<T>
where
    T: Clone,
{
    inner: T,
}

pub struct Unit;

pub struct Meters(pub f64);
