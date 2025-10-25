// Quick test to understand line handling
fn main() {
    let line1 = "  hello world  \n";
    let line2 = "  hello world  ";
    
    println!("Line 1: '{}'", line1);
    println!("Line 1 len: {}", line1.len());
    println!("Line 1 chars: {}", line1.chars().count());
    
    let trimmed1 = line1.trim_end_matches('\n');
    println!("Trimmed 1: '{}'", trimmed1);
    println!("Trimmed 1 chars: {}", trimmed1.chars().count());
    println!("Cursor would be at: {}", trimmed1.chars().count().saturating_sub(1));
    
    println!("\nLine 2: '{}'", line2);
    println!("Line 2 chars: {}", line2.chars().count());
    let col = if line2.chars().count() > 0 { line2.chars().count() - 1 } else { 0 };
    println!("Cursor would be at: {}", col);
}
