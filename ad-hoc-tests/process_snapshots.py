#!/usr/bin/env python3
import re
import os
from pathlib import Path

def parse_snapshot(snapshot_path):
    """Parse a snapshot file and extract cursor position and buffer content."""
    with open(snapshot_path, 'r') as f:
        content = f.read()

    # Extract cursor position
    cursor_match = re.search(r'Cursor: (\d+):(\d+)', content)
    if not cursor_match:
        return None, None

    line, col = int(cursor_match.group(1)), int(cursor_match.group(2))

    # Extract buffer content (everything after "Buffer:\n")
    buffer_match = re.search(r'Buffer:\n(.*)', content, re.DOTALL)
    if not buffer_match:
        return None, None

    buffer_content = buffer_match.group(1)

    # Remove cursor markers [c] or [l] or any single char in brackets
    # The marker shows cursor position but we don't need it in the assertion
    buffer_content = re.sub(r'\[.\]', '', buffer_content)

    # Remove trailing newline that's part of the snapshot format
    if buffer_content.endswith('\n'):
        buffer_content = buffer_content[:-1]

    return buffer_content, (line, col)

def process_test_file(test_file_path):
    """Process a test file and generate refactored assertions."""
    test_name = Path(test_file_path).stem
    snapshots_dir = Path('tests/snapshots')

    with open(test_file_path, 'r') as f:
        test_content = f.read()

    # Find all test functions and their snapshot assertions
    test_pattern = r'#\[test\]\s+fn\s+(\w+)\(\)\s*\{[^}]+assert_snapshot!\((?:"([^"]+)",\s*)?test\.snapshot_state\(\)\);'

    replacements = []

    for match in re.finditer(test_pattern, test_content, re.DOTALL):
        test_fn_name = match.group(1)
        snapshot_name = match.group(2) if match.group(2) else test_fn_name

        # Remove 'test_' prefix from test name to match snapshot filename
        if snapshot_name.startswith('test_'):
            snapshot_name = snapshot_name[5:]

        # Find corresponding snapshot file
        snapshot_file = snapshots_dir / f"{test_name}__{snapshot_name}.snap"

        if not snapshot_file.exists():
            print(f"Warning: Snapshot not found for {test_fn_name}: {snapshot_file}")
            continue

        buffer_content, cursor = parse_snapshot(snapshot_file)
        if buffer_content is None:
            print(f"Warning: Could not parse snapshot for {test_fn_name}")
            continue

        line, col = cursor

        # Escape special characters in buffer content for Rust string
        buffer_escaped = buffer_content.replace('\\', '\\\\').replace('"', '\\"')

        # Generate replacement code
        old_assertion = match.group(0)
        # Find just the assert_snapshot line
        assert_match = re.search(r'assert_snapshot!\((?:"[^"]+",\s*)?test\.snapshot_state\(\)\);', old_assertion)
        if assert_match:
            old_assert = assert_match.group(0)
            new_assert = f'assert_eq!(test.buffer_content(), "{buffer_escaped}\\n");\n    test.assert_cursor({line}, {col});'
            replacements.append((old_assert, new_assert))

    return replacements

# Process both test files
for test_file in ['tests/motion_edge_cases_test.rs', 'tests/motion_bounds_test.rs']:
    print(f"\nProcessing {test_file}...")
    replacements = process_test_file(test_file)
    print(f"Found {len(replacements)} tests to convert")

    if replacements:
        with open(test_file, 'r') as f:
            content = f.read()

        for old, new in replacements:
            content = content.replace(old, new)

        # Remove the insta import
        content = re.sub(r'use insta::assert_snapshot;\n', '', content)

        with open(test_file, 'w') as f:
            f.write(content)

        print(f"Converted {len(replacements)} tests in {test_file}")
