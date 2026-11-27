#!/usr/bin/env python3
"""Map a wasm file offset to its function name (name section + code section).

    python3 scripts/wasm_symbolize.py <file.wasm> 0xf1db52 [more offsets...]

The browser reports panics as `page_bg.wasm:0x<offset>` where the offset is the
byte position in the module file. Every function body lives in the code section;
its `name` comes from the name section that wasm-bindgen/wasm-opt keep. This
prints `<offset> -> <function name>` for each requested offset, which is the
artifact the reports need.
"""

import sys
from pathlib import Path


def leb(data, pos):
    result = 0
    shift = 0
    while True:
        byte = data[pos]
        pos += 1
        result |= (byte & 0x7F) << shift
        if byte < 0x80:
            return result, pos
        shift += 7


def sections(data):
    pos = 8  # magic + version
    out = []
    while pos < len(data):
        sid = data[pos]
        pos += 1
        size, pos = leb(data, pos)
        start = pos
        out.append((sid, start, size))
        pos = start + size
    return out


def function_names(data):
    names = {}
    for sid, start, size in sections(data):
        if sid != 0:
            continue
        end = start + size
        pos = start
        while pos < end:
            if pos >= end:
                break
            sub = data[pos]
            pos += 1
            sub_size, pos = leb(data, pos)
            sub_end = pos + sub_size
            if sub == 1:  # function-name subsection
                count, p = leb(data, pos)
                for _ in range(count):
                    idx, p = leb(data, p)
                    nlen, p = leb(data, p)
                    names[idx] = data[p : p + nlen].decode("utf-8", "replace")
                    p += nlen
            pos = sub_end
    return names


def code_ranges(data):
    """(start_offset, end_offset, function_index) for every code entry."""
    ranges = []
    for sid, start, size in sections(data):
        if sid != 10:  # code section
            continue
        pos = start
        count, pos = leb(data, pos)
        for index in range(count):
            body_size, pos = leb(data, pos)
            body_start = pos
            ranges.append((body_start, body_start + body_size, index))
            pos = body_start + body_size
    return ranges


def main():
    path = Path(sys.argv[1])
    offsets = [int(value, 16) if value.lower().startswith("0x") else int(value) for value in sys.argv[2:]]
    data = path.read_bytes()
    names = function_names(data)
    ranges = code_ranges(data)
    for offset in offsets:
        hit = next((r for r in ranges if r[0] <= offset < r[1]), None)
        if hit is None:
            print(f"{offset:#x} -> (outside the code section)")
            continue
        name = names.get(hit[2], f"func[{hit[2]}]")
        print(f"{offset:#x} -> {name}   (func index {hit[2]}, body {hit[0]:#x}..{hit[1]:#x})")


if __name__ == "__main__":
    main()
