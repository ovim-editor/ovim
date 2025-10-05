#!/usr/bin/env python3
"""Refactor change_operations_test.rs to use direct assertions instead of snapshots."""

import re
from pathlib import Path

def parse_snapshot(content):
    """Parse a snapshot file and extract cursor position and buffer content."""
    lines = content.strip().split('\n')

    # Extract cursor position from "Cursor: LINE:COL"
    cursor_line = None
    for line in lines:
        if line.startswith('Cursor:'):
            match = re.match(r'Cursor:\s*(\d+):(\d+)', line)
            if match:
                cursor_line = (int(match.group(1)), int(match.group(2)))
                break

    # Extract buffer content (after "Buffer:" line)
    buffer_start = None
    for i, line in enumerate(lines):
        if line.startswith('Buffer:'):
            buffer_start = i + 1
            break

    if buffer_start is None or cursor_line is None:
        return None, None

    # Get buffer lines (everything after "Buffer:")
    buffer_lines = lines[buffer_start:]

    # Remove cursor markers [c] from buffer
    cleaned_lines = []
    for line in buffer_lines:
        # Remove [X] pattern (cursor marker on character X)
        cleaned = re.sub(r'\[(.)\]', r'\1', line)
        # Remove [ ] pattern (cursor on space)
        cleaned = re.sub(r'\[ \]', ' ', cleaned)
        cleaned_lines.append(cleaned)

    # Join with newlines and add final newline
    buffer_content = '\n'.join(cleaned_lines) + '\n'

    return buffer_content, cursor_line

# Parse all snapshots
snapshot_dir = Path('tests/snapshots')
test_data = {}

for snapshot_file in sorted(snapshot_dir.glob('change_operations_test__*.snap')):
    # Extract test name from filename
    # e.g., "change_operations_test__cw_change_word.snap" -> "test_cw_change_word"
    test_name = "test_" + snapshot_file.stem.replace('change_operations_test__', '')

    with open(snapshot_file, 'r') as f:
        content = f.read()

    buffer, cursor = parse_snapshot(content)
    if buffer and cursor:
        test_data[test_name] = (buffer, cursor[0], cursor[1])

print(f"Parsed {len(test_data)} snapshots")

# Read the test file
with open('tests/change_operations_test.rs', 'r') as f:
    content = f.read()

# Remove the insta import
content = content.replace('use insta::assert_snapshot;\n', '')

# Replace each assertion
for test_name, (expected_buffer, line, col) in sorted(test_data.items()):
    # Escape the expected buffer for use in a string literal
    escaped_buffer = expected_buffer.replace('\\', '\\\\').replace('"', '\\"').replace('\n', '\\n')

    # Build the new assertion
    new_assertion = f'assert_eq!(test.buffer_content(), "{escaped_buffer}");\n    test.assert_cursor({line}, {col});'

    # Find and replace the snapshot assertion for this test
    # Look for the pattern after the test function
    pattern = rf'(fn {re.escape(test_name)}\(\).*?)(assert_snapshot!\(test\.snapshot_state\(\)\);)'

    def replace_fn(match):
        return match.group(1) + new_assertion

    new_content, count = re.subn(pattern, replace_fn, content, flags=re.DOTALL)
    if count > 0:
        content = new_content
        print(f"Converted {test_name}")
    else:
        print(f"WARNING: Could not find test {test_name}")

# Write the modified content
with open('tests/change_operations_test.rs', 'w') as f:
    f.write(content)

print(f"\nDone! Converted {len(test_data)} tests")
print("Removed import: use insta::assert_snapshot;")
