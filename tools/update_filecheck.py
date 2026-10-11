#!/usr/bin/env python3
"""Bless fixture output: rewrite the filecheck directives of failing fixtures.

Runs `cargo test --test fixtures`, and for every fixture whose checks no longer
match, replaces its check:/nextln:/sameln:/not:/unordered: lines with lines
generated from the actual output. Files are updated in place.

    tools/update_filecheck.py                    # bless every failing fixture
    tools/update_filecheck.py 'view/*.isle'      # only fixtures matching a glob
    tools/update_filecheck.py tests/fixtures/stages
    tools/update_filecheck.py -n                 # show diffs, write nothing

Globs are relative to tests/fixtures. A directory means every .isle below it.
The generated block replaces the old directives at the position of the first
one, and each blessed file is re-run to confirm it passes; if it does not, the
original is restored.
"""

import argparse
import difflib
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "tests" / "fixtures"
DIRECTIVE = re.compile(r"^\s*;\s*(check|nextln|sameln|not|unordered):")
FAIL = re.compile(r"^FAIL (.+)$", re.MULTILINE)


def run_suite(extra=(), actual_dir=None):
    env = dict(os.environ)
    if actual_dir is not None:
        env["FIXTURES_ACTUAL_DIR"] = str(actual_dir)
    return subprocess.run(
        ["cargo", "test", "--test", "fixtures", "--", *extra],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
    )


def failed_names(result):
    return FAIL.findall(result.stderr)


def selected(patterns):
    if not patterns:
        return None
    chosen = set()
    for pattern in patterns:
        pattern = re.sub(r"^(\./)?tests/fixtures/", "", pattern)
        if (FIXTURES / pattern).is_dir():
            pattern = pattern.rstrip("/") + "/**/*.isle"
        chosen.update(p.relative_to(FIXTURES).as_posix() for p in FIXTURES.glob(pattern))
    return chosen


def pattern_line(text):
    return text.strip().replace("$", "$$")


def generate(output):
    lines = []
    previous_blank = True
    for line in output.splitlines():
        if not line.strip():
            previous_blank = True
            continue
        keyword = "check" if previous_blank else "nextln"
        lines.append(f"; {keyword}: {pattern_line(line)}")
        previous_blank = False
    return lines


def rewrite(source, output):
    lines = source.splitlines()
    first = next((i for i, line in enumerate(lines) if DIRECTIVE.match(line)), None)
    if first is None:
        return None
    kept = lines[:first] + generate(output) + [l for l in lines[first:] if not DIRECTIVE.match(l)]
    return "\n".join(kept) + ("\n" if source.endswith("\n") else "")


def directive_count(text):
    return sum(1 for line in text.splitlines() if DIRECTIVE.match(line))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("globs", nargs="*", help="fixture globs, relative to tests/fixtures")
    parser.add_argument("-n", "--dry-run", action="store_true", help="show diffs without writing")
    args = parser.parse_args()

    chosen = selected(args.globs)
    if chosen is not None and not chosen:
        sys.exit("no fixtures match " + " ".join(args.globs))

    with tempfile.TemporaryDirectory() as tmp:
        actual = Path(tmp)
        result = run_suite(actual_dir=actual)
        failures = failed_names(result)
        if not failures and result.returncode != 0:
            sys.exit(result.stderr)
        if not failures:
            print("all fixtures pass, nothing to bless")
            return

        blessed, reverted, skipped = [], [], []
        for name in failures:
            if chosen is not None and name not in chosen:
                continue
            output_file = actual / name
            if not output_file.is_file():
                skipped.append(name)
                continue
            path = FIXTURES / name
            source = path.read_text()
            updated = rewrite(source, output_file.read_text())
            if updated is None or updated == source:
                skipped.append(name)
                continue
            before, after = directive_count(source), directive_count(updated)
            if args.dry_run:
                diff = difflib.unified_diff(source.splitlines(), updated.splitlines(), f"a/{name}", f"b/{name}", lineterm="")
                print("\n".join(diff))
                blessed.append((name, before, after))
                continue
            path.write_text(updated)
            if name in failed_names(run_suite([name])):
                path.write_text(source)
                reverted.append(name)
            else:
                blessed.append((name, before, after))

    for name, before, after in blessed:
        verb = "would bless" if args.dry_run else "blessed"
        print(f"{verb} {name} ({before} -> {after} directives)")
    for name in reverted:
        print(f"reverted {name}: the generated checks did not pass, edit it by hand")
    for name in skipped:
        print(f"skipped {name}: not a check mismatch (invariant, oracle, bad directive, or no change)")
    unmatched = [n for n in failures if chosen is not None and n not in chosen]
    if unmatched:
        print(f"{len(unmatched)} other failing fixture(s) outside the given globs")
    if reverted or skipped:
        sys.exit(1)


if __name__ == "__main__":
    main()
