#!/usr/bin/env python3
"""Parse snapshot files and extract expected buffer content and cursor position."""

import os
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
        # Remove [X] pattern (cursor marker)
        cleaned = re.sub(r'\[(.)\]', r'\1', line)
        # Remove [ ] pattern (space cursor marker)
        cleaned = re.sub(r'\[ \]', ' ', cleaned)
        cleaned_lines.append(cleaned)

    # Join with newlines and add final newline
    buffer_content = '\n'.join(cleaned_lines) + '\n'

    return buffer_content, cursor_line

# Directory containing snapshots
snapshot_dir = Path('tests/snapshots')

# Parse all change_operations_test snapshots
test_data = {}
for snapshot_file in sorted(snapshot_dir.glob('change_operations_test__*.snap')):
    # Extract test name from filename
    test_name = snapshot_file.stem  # Remove .snap extension

    with open(snapshot_file, 'r') as f:
        content = f.read()

    buffer, cursor = parse_snapshot(content)
    if buffer and cursor:
        test_data[test_name] = (buffer, cursor[0], cursor[1])
        print(f"{test_name}: cursor={cursor}, len={len(buffer)}")

# Generate Python code
print("\n\n# Test data dictionary:")
print("test_data = {")
for test_name, (buffer, line, col) in test_data.items():
    # Escape the buffer content for Python string
    escaped = repr(buffer)
    print(f'    "{test_name}": ({escaped}, {line}, {col}),')
print("}")
