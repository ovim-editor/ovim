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
    lines = f.readlines()

# Remove the insta import line
new_lines = []
for line in lines:
    if line.strip() != 'use insta::assert_snapshot;':
        new_lines.append(line)
lines = new_lines

# Now replace assert_snapshot lines
new_lines = []
converted = 0
for i, line in enumerate(lines):
    if 'assert_snapshot!(test.snapshot_state());' in line:
        # Find which test this belongs to by looking backwards
        test_name = None
        for j in range(i-1, max(0, i-50), -1):
            if 'fn test_' in lines[j]:
                match = re.search(r'fn (test_\w+)\(\)', lines[j])
                if match:
                    test_name = match.group(1)
                    break

        if test_name and test_name in test_data:
            buffer, line_num, col = test_data[test_name]
            # Escape the buffer for Rust string literal
            escaped = buffer.replace('\\', '\\\\').replace('"', '\\"').replace('\n', '\\n')

            # Get the indentation from the original line
            indent = line[:len(line) - len(line.lstrip())]

            # Write the new assertions
            new_lines.append(f'{indent}assert_eq!(test.buffer_content(), "{escaped}");\n')
            new_lines.append(f'{indent}test.assert_cursor({line_num}, {col});\n')
            converted += 1
            print(f"Converted {test_name}")
        else:
            print(f"WARNING: No snapshot data for test {test_name}")
            new_lines.append(line)
    else:
        new_lines.append(line)

# Write the modified content
with open('tests/change_operations_test.rs', 'w') as f:
    f.writelines(new_lines)

print(f"\nDone! Converted {converted} test assertions")
print("Removed import: use insta::assert_snapshot;")
