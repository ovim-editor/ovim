#!/usr/bin/env python3
"""
Convert snapshot-based tests to direct assertion tests.
Parses snapshot files and generates test assertions.
"""
import re
from pathlib import Path

def parse_snapshot(snapshot_path):
    """Parse a snapshot file and return dict with buffer, cursor, mode"""
    try:
        with open(snapshot_path, 'r') as f:
            content = f.read()
    except FileNotFoundError:
        return None

    cursor_match = re.search(r'Cursor: (\d+):(\d+)', content)
    if not cursor_match:
        return None

    cursor_line = int(cursor_match.group(1))
    cursor_col = int(cursor_match.group(2))

    mode_match = re.search(r'Mode: (\w+)', content)
    mode = mode_match.group(1) if mode_match else "Normal"

    buffer_match = re.search(r'Buffer:\n(.+?)(?:\n\n|\n---|\Z)', content, re.DOTALL)
    if not buffer_match:
        return None

    buffer_text = buffer_match.group(1).rstrip()
    buffer_lines = buffer_text.split('\n')

    processed_lines = []
    for line in buffer_lines:
        cleaned = re.sub(r'\[(.?)\]', r'\1', line)
        processed_lines.append(cleaned)

    buffer_content = '\n'.join(processed_lines) + '\n'

    return {
        'buffer': buffer_content,
        'cursor_line': cursor_line,
        'cursor_col': cursor_col,
        'mode': mode
    }

def escape_string(s):
    """Escape a string for Rust string literal"""
    return s.replace('\\', '\\\\').replace('"', '\\"').replace('\n', '\\n')

def convert_test_file(test_file_path, snapshot_dir):
    """Convert a test file from snapshot assertions to direct assertions"""
    with open(test_file_path, 'r') as f:
        content = f.read()

    # Remove the insta import
    content = re.sub(r'use insta::assert_snapshot;\n', '', content)

    # Find all test functions with assert_snapshot
    test_pattern = r'#\[test\]\s+fn\s+(\w+)\([^)]*\)\s*\{(.*?)^\}'
    matches = list(re.finditer(test_pattern, content, re.MULTILINE | re.DOTALL))

    # Process matches in reverse order to preserve positions
    for match in reversed(matches):
        test_name = match.group(1)
        test_body = match.group(2)

        # Check if it has assert_snapshot
        if 'assert_snapshot!' in test_body:
            # Find the snapshot file
            snapshot_name = f"{test_file_path.stem}__{test_name}.snap"
            snapshot_path = snapshot_dir / snapshot_name

            snapshot_data = parse_snapshot(snapshot_path)

            if snapshot_data:
                # Replace assert_snapshot with direct assertions
                buffer_escaped = escape_string(snapshot_data['buffer'])
                new_assertions = f'\n    assert_eq!(test.buffer_content(), "{buffer_escaped}");\n    test.assert_cursor({snapshot_data["cursor_line"]}, {snapshot_data["cursor_col"]});'

                # Replace the assert_snapshot line
                new_body = re.sub(
                    r'\n\s*assert_snapshot!\(test\.snapshot_state\(\)\);',
                    new_assertions,
                    test_body
                )

                # Replace in content
                content = content[:match.start(2)] + new_body + content[match.end(2):]

    return content

# Main execution
test_files = [
    Path('/workspace/tests/register_operations_test.rs'),
    Path('/workspace/tests/mark_test.rs')
]
snapshot_dir = Path('/workspace/tests/snapshots')

for test_file in test_files:
    print(f"Processing {test_file.name}...")
    converted = convert_test_file(test_file, snapshot_dir)
    # Write to a .converted file first for review
    output_path = test_file.with_suffix('.rs.converted')
    with open(output_path, 'w') as f:
        f.write(converted)
    print(f"  Wrote to {output_path}")

print("Done!")
