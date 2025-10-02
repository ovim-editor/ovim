fn main() {
    let x = 42;
    println!("Hello, world!");
    // This is a comment
    let name = "Rust";
}

struct Point {
    x: i32,
    y: i32,
}

impl Point {
    fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}
