fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn multiply(x: i32, y: i32) -> i32 {
    x * y
}

fn main() {
    let result = add(3, 4);
    let product = multiply(5, 6);
    println!("Result: {}, Product: {}", result, product);
}
