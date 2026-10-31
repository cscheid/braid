#!/usr/bin/env python3
"""Inclusive-time, per-thread view of a samply profile (stdlib only).

Reads `samply record --unstable-presymbolicate` output: the Firefox Profiler
columnar JSON (.json.gz) plus its `.syms.json` sidecar. Self-time tables
scatter a slow operation across dozens of leaf functions; this answers "which
operation is the time under?" instead.

    analyze_inclusive.py <profile.json.gz> [--top N] [--match REGEX]

For each thread it prints the sample count, the time span, and the top
functions by inclusive samples whose name matches --match. Symbols in system
libraries stay unresolved (use macOS `sample` for those).
"""
import argparse
import gzip
import json
import re
from bisect import bisect_right
from collections import Counter
from pathlib import Path


def load_syms(path):
    data = json.load(open(path))
    strings = data["string_table"]
    modules = {}
    for entry in data["data"]:
        table = sorted(entry.get("symbol_table", []), key=lambda s: s["rva"])
        if table:
            modules[entry["debug_name"]] = (
                [s["rva"] for s in table],
                [s["size"] for s in table],
                [strings[s["symbol"]] for s in table],
            )
    return modules


def resolve(modules, module, raw):
    if not raw.startswith("0x") or module not in modules:
        return raw
    rvas, sizes, syms = modules[module]
    addr = int(raw, 16)
    i = bisect_right(rvas, addr) - 1
    return syms[i] if i >= 0 and addr < rvas[i] + sizes[i] else raw


def func_names(thread, modules):
    strings = thread["stringArray"]
    funcs = thread["funcTable"]
    resources = thread.get("resourceTable", {}).get("name", [])
    names = []
    for raw, res in zip(funcs["name"], funcs["resource"]):
        module = strings[resources[res]] if res is not None and 0 <= res < len(resources) else ""
        names.append(resolve(modules, module, strings[raw]))
    return names


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("profile", type=Path)
    ap.add_argument("--top", type=int, default=30)
    ap.add_argument("--match", default=r"braid|samod|automerge::")
    args = ap.parse_args()

    syms = args.profile.with_name(args.profile.name.removesuffix(".gz") + ".syms.json")
    modules = load_syms(syms) if syms.exists() else {}
    profile = json.load(gzip.open(args.profile, "rt"))
    pattern = re.compile(args.match)

    for thread in profile["threads"]:
        samples = thread["samples"]
        stacks = samples.get("stack", [])
        if not stacks:
            continue
        names = func_names(thread, modules)
        frame_func = thread["frameTable"]["func"]
        st = thread["stackTable"]
        inclusive = Counter()
        memo = {}

        def funcs_of(s):
            """The set of function ids on stack `s` (memoized up the prefix chain)."""
            if s is None:
                return frozenset()
            if s not in memo:
                memo[s] = funcs_of(st["prefix"][s]) | {frame_func[st["frame"][s]]}
            return memo[s]

        for s in stacks:
            for f in funcs_of(s):
                inclusive[f] += 1

        times = samples.get("time") or []
        if not times and "timeDeltas" in samples:
            t, times = 0, []
            for d in samples["timeDeltas"]:
                t += d
                times.append(t)
        span = f"{times[0]:.0f}..{times[-1]:.0f}ms" if times else "?"
        print(f"\n== thread {thread.get('name')!r} samples={len(stacks)} span={span}")
        shown = 0
        for f, count in inclusive.most_common():
            if pattern.search(names[f]):
                print(f"{count:7d}  {names[f][:130]}")
                shown += 1
                if shown >= args.top:
                    break


if __name__ == "__main__":
    main()
