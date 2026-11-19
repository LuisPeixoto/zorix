"""Profile a CSV file (a path, or text on standard input) with the standard library only. Prints one JSON object."""
import csv
import json
import statistics
import sys
from collections import Counter
from datetime import datetime

MAX_ROWS = 200_000


def kind_of(values):
    present = [v for v in values if v != ""]
    if not present:
        return "text"
    for name, test in (("integer", int), ("number", float)):
        try:
            for v in present:
                test(v)
            return name
        except ValueError:
            pass
    for fmt in ("%Y-%m-%d", "%Y-%m-%dT%H:%M:%S", "%d/%m/%Y"):
        try:
            for v in present:
                datetime.strptime(v, fmt)
            return "date"
        except ValueError:
            pass
    return "text"


def read_rows(handle):
    reader = csv.reader(handle)
    header = next(reader, None)
    if header is None:
        return None, [], False
    rows = []
    truncated = False
    for row in reader:
        if len(rows) >= MAX_ROWS:
            truncated = True
            break
        rows.append(row + [""] * (len(header) - len(row)))
    return header, rows, truncated


def main():
    # With a path, read that file (when the workspace is mounted); without one, read standard input.
    if len(sys.argv) > 1:
        with open(sys.argv[1], newline="", encoding="utf-8-sig") as handle:
            header, rows, truncated = read_rows(handle)
    else:
        header, rows, truncated = read_rows(sys.stdin)
    if header is None:
        print(json.dumps({"error": "the file is empty"}))
        return 1
    columns = []
    for index, name in enumerate(header):
        values = [row[index].strip() for row in rows]
        kind = kind_of(values)
        info = {
            "name": name,
            "type": kind,
            "missing": sum(1 for v in values if v == ""),
            "distinct": len(set(values)),
        }
        if kind in ("integer", "number"):
            numbers = [float(v) for v in values if v != ""]
            info.update(
                min=min(numbers),
                max=max(numbers),
                mean=round(statistics.fmean(numbers), 6),
                median=statistics.median(numbers),
            )
        elif kind == "text":
            info["common"] = [[v, n] for v, n in Counter(v for v in values if v).most_common(5)]
        columns.append(info)
    print(json.dumps({"rows": len(rows), "truncated": truncated, "columns": columns}))
    return 0


sys.exit(main())
