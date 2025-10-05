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

    # Find all test functions
    test_functions = re.finditer(r'#\[test\]\s+fn\s+(\w+)\([^)]*\)\s*\{', content)

    test_ranges = []
    for match in test_functions:
        test_name = match.group(1)
        start_pos = match.start()

        # Find the end of this test function (next #[test] or end of file)
        next_test = re.search(r'#\[test\]', content[match.end():])
        if next_test:
            end_pos = match.end() + next_test.start()
        else:
            end_pos = len(content)

        test_ranges.append((test_name, start_pos, end_pos))

    # Process in reverse order to preserve positions
    for test_name, start_pos, end_pos in reversed(test_ranges):
        test_body = content[start_pos:end_pos]

        # Find assert_snapshot in this test
        assert_match = re.search(r'(\s*)assert_snapshot!\(test\.snapshot_state\(\)\);', test_body)
        if not assert_match:
            continue

        # Remove 'test_' prefix for snapshot name
        snapshot_name = test_name
        if snapshot_name.startswith('test_'):
            snapshot_name = snapshot_name[5:]

        # Find the snapshot file
        snapshot_file_name = f"{test_file_path.stem}__{snapshot_name}.snap"
        snapshot_path = snapshot_dir / snapshot_file_name

        snapshot_data = parse_snapshot(snapshot_path)
        if not snapshot_data:
            print(f"  Warning: Could not parse snapshot for {test_name}")
            continue

        # Build replacement
        indent = assert_match.group(1)
        buffer_escaped = escape_string(snapshot_data['buffer'])
        new_assertions = f'{indent}assert_eq!(test.buffer_content(), "{buffer_escaped}");\n{indent}test.assert_cursor({snapshot_data["cursor_line"]}, {snapshot_data["cursor_col"]});'

        # Replace in test body
        new_test_body = test_body[:assert_match.start()] + new_assertions + test_body[assert_match.end():]

        # Replace in content
        content = content[:start_pos] + new_test_body + content[end_pos:]

    return content

# Main execution
test_files = [
    Path('/workspace/tests/motion_edge_cases_test.rs'),
    Path('/workspace/tests/motion_bounds_test.rs')
]
snapshot_dir = Path('/workspace/tests/snapshots')

for test_file in test_files:
    print(f"Processing {test_file.name}...")
    converted = convert_test_file(test_file, snapshot_dir)
    # Write directly to the test file
    with open(test_file, 'w') as f:
        f.write(converted)
    print(f"  Converted {test_file.name}")

print("Done!")
