#!/usr/bin/env bash
# Generate benchmark test files for performance testing

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCH_DIR="$SCRIPT_DIR/benchmarks"

# Create benchmarks directory
mkdir -p "$BENCH_DIR"

echo "Generating benchmark files in $BENCH_DIR..."

# Generate small.txt (1K lines)
echo "Creating small.txt (1K lines)..."
seq 1 1000 > "$BENCH_DIR/small.txt"

# Generate medium.txt (10K lines)
echo "Creating medium.txt (10K lines)..."
seq 1 10000 > "$BENCH_DIR/medium.txt"

# Generate large.txt (100K lines)
echo "Creating large.txt (100K lines)..."
seq 1 100000 > "$BENCH_DIR/large.txt"

# Generate huge.txt (500K lines)
echo "Creating huge.txt (500K lines)..."
seq 1 500000 > "$BENCH_DIR/huge.txt"

# Generate a realistic Rust source file (medium sized)
echo "Creating rust_medium.rs (realistic Rust code, ~5K lines)..."
cat > "$BENCH_DIR/rust_medium.rs" << 'EOF'
// Auto-generated benchmark file for syntax highlighting performance testing
use std::collections::HashMap;
use std::sync::Arc;

EOF

for i in {1..100}; do
  cat >> "$BENCH_DIR/rust_medium.rs" << EOF
/// Module $i documentation
pub mod module_$i {
    use super::*;

    /// Struct for module $i
    pub struct Data$i {
        pub id: u64,
        pub name: String,
        pub value: Option<f64>,
        pub tags: Vec<String>,
    }

    impl Data$i {
        /// Create a new instance
        pub fn new(id: u64, name: String) -> Self {
            Self {
                id,
                name,
                value: None,
                tags: Vec::new(),
            }
        }

        /// Process the data
        pub fn process(&mut self) -> Result<(), String> {
            if self.id == 0 {
                return Err("Invalid ID".to_string());
            }

            self.value = Some(self.id as f64 * 3.14159);
            self.tags.push(format!("processed_{}", self.id));

            Ok(())
        }

        /// Get the computed value
        pub fn get_value(&self) -> Option<f64> {
            self.value
        }
    }

    /// Function to demonstrate control flow
    pub fn compute_$i(input: i32) -> i32 {
        let mut result = 0;

        for i in 0..input {
            if i % 2 == 0 {
                result += i;
            } else {
                result -= i;
            }
        }

        match result {
            x if x > 100 => x / 2,
            x if x < -100 => x * 2,
            x => x,
        }
    }
}

EOF
done

# Generate a large Rust file (100K+ lines)
echo "Creating rust_large.rs (realistic Rust code, ~100K lines)..."
cat > "$BENCH_DIR/rust_large.rs" << 'EOF'
// Auto-generated large benchmark file for performance testing
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

EOF

for i in {1..2000}; do
  cat >> "$BENCH_DIR/rust_large.rs" << EOF
pub struct Struct$i {
    field_a: u64,
    field_b: String,
    field_c: Vec<i32>,
}

impl Struct$i {
    pub fn new() -> Self {
        Self {
            field_a: $i,
            field_b: String::from("struct_$i"),
            field_c: vec![$(($i % 10)), $(($i % 20)), $(($i % 30))],
        }
    }

    pub fn method_a(&self) -> u64 { self.field_a }
    pub fn method_b(&self) -> &str { &self.field_b }
    pub fn method_c(&self) -> &[i32] { &self.field_c }
}

EOF
done

# Generate stats
echo ""
echo "=== Benchmark files generated ==="
echo ""
for file in "$BENCH_DIR"/*.{txt,rs}; do
    if [ -f "$file" ]; then
        lines=$(wc -l < "$file")
        size=$(du -h "$file" | cut -f1)
        echo "$(basename "$file"): $lines lines, $size"
    fi
done

echo ""
echo "Done! Benchmark files are in: $BENCH_DIR"
