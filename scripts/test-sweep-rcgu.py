#!/usr/bin/env -S python3 -I
"""Checks scripts/sweep-rcgu.py on a small crate in a temporary folder (LIO-676).

Usage: scripts/test-sweep-rcgu.py      (macOS; about a minute; prints "ok" at the end)

The crate (a binary using a library from a path dependency) is built
incrementally a few times, so deps/ fills with object files of earlier builds.
It checks that:

* a sweep deletes the unused objects and keeps every one a binary points at;
* nothing is rebuilt after it (cargo says Fresh), an edit rebuilds
  incrementally, and a panic's backtrace still has its file:line;
* the dry run deletes nothing;
* a profile folder whose Cargo lock is held, and a target folder with a file
  open below it, are left alone.
"""

import fcntl
import os
import re
import subprocess
import sys
import tempfile

SWEEP = os.path.join(os.path.dirname(os.path.abspath(__file__)), "sweep-rcgu.py")
os.environ["PATH"] += os.pathsep + os.path.expanduser("~/.cargo/bin")


def sh(*cmd, cwd, env=None, check=True):
    r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, env={**os.environ, **(env or {})})
    if check and r.returncode != 0:
        sys.exit(f"failed: {' '.join(cmd)}\n{r.stdout}\n{r.stderr}")
    return r


def objects(target):
    return sorted(os.path.join(dp, f) for dp, _, fns in os.walk(target) for f in fns
                  if f.endswith(".rcgu.o") and "/incremental/" not in dp + "/")


def referenced(target):
    keep = set()
    for dp, dns, fns in os.walk(target):
        dns[:] = [d for d in dns if d != "incremental" and not d.endswith(".dSYM")]
        for f in fns:
            p = os.path.join(dp, f)
            if f.endswith((".o", ".rlib", ".rmeta", ".d")) or os.path.islink(p):
                continue
            with open(p, "rb") as fh:
                if fh.read(4) != b"\xcf\xfa\xed\xfe":
                    continue
            for line in sh("nm", "-ap", p, cwd=target).stdout.splitlines():
                m = re.search(r" OSO (.+)$", line)
                if m and not m.group(1).endswith(")"):
                    keep.add(os.path.realpath(m.group(1)))
    return keep


def sweep(*args, cwd):
    return sh(sys.executable, "-I", SWEEP, *args, cwd=cwd).stdout


def edit(main_rs, k):
    s = open(main_rs).read()
    s = re.sub(r"const K: u64 = \d+;", f"const K: u64 = {k};", s)
    open(main_rs, "w").write(s)


def main():
    with tempfile.TemporaryDirectory(prefix="sweep-rcgu-test-") as tmp:
        tmp = os.path.realpath(tmp)
        os.makedirs(f"{tmp}/dep/src")
        os.makedirs(f"{tmp}/demo/src")
        open(f"{tmp}/dep/Cargo.toml", "w").write('[package]\nname = "dep"\nversion = "0.1.0"\nedition = "2021"\n')
        open(f"{tmp}/dep/src/lib.rs", "w").write(
            "pub fn g(x: u64) -> u64 { (0..x).map(|v| v * 3).sum() }\n")
        open(f"{tmp}/demo/Cargo.toml", "w").write(
            '[package]\nname = "demo"\nversion = "0.1.0"\nedition = "2021"\n\n'
            '[dependencies]\ndep = { path = "../dep" }\n\n[workspace]\n')
        mods = "".join(f"pub mod m{i} {{ pub fn f(x: u64) -> u64 {{ (0..x).map(|v| v.wrapping_mul({i} + 1)).sum::<u64>() + {i} }} }}\n"
                       for i in range(40))
        calls = "".join(f"t += m{i}::f({i}); " for i in range(40))
        open(f"{tmp}/demo/src/main.rs", "w").write(
            mods + "const K: u64 = 0;\n"
            f"fn main() {{ let mut t = K + dep::g(3); {calls}println!(\"{{t}}\"); }}\n"
            "#[cfg(test)] mod tests {\n"
            "    fn inner() { panic!(\"boom\") }\n"
            "    #[test] fn t() { assert!(super::m1::f(3) > 0); if std::env::var(\"BOOM\").is_ok() { inner(); } }\n"
            "}\n")
        demo, target = f"{tmp}/demo", f"{tmp}/demo/target"
        env = {"CARGO_TARGET_DIR": target}
        env.pop("CARGO_INCREMENTAL", None)
        os.environ.pop("CARGO_INCREMENTAL", None)
        os.environ.pop("CARGO_TARGET_DIR", None)
        for k in range(1, 4):
            edit(f"{demo}/src/main.rs", k)
            sh("cargo", "test", "--no-run", "-q", cwd=demo)
            sh("cargo", "build", "-q", cwd=demo)
        before = objects(target)
        keep = referenced(target)
        assert len(before) > len(keep) > 0, (len(before), len(keep))

        out = sweep(target, cwd=tmp)
        assert "would delete" in out and objects(target) == before, out

        with open(f"{target}/debug/.cargo-build-lock") as held:
            fcntl.flock(held, fcntl.LOCK_EX)
            out = sweep("--delete", target, cwd=tmp)
            assert "busy (Cargo holds its lock)" in out and objects(target) == before, out

        with open(f"{target}/debug/demo"):
            out = sweep("--delete", target, cwd=tmp)
            assert "busy (a file below it is open)" in out and objects(target) == before, out

        out = sweep("--delete", target, cwd=tmp)
        after = objects(target)
        assert set(os.path.realpath(p) for p in after) == keep & set(os.path.realpath(p) for p in before), out
        assert len(after) < len(before), out

        verbose = sh("cargo", "build", "-v", cwd=demo).stderr
        assert "Compiling" not in verbose and "Fresh demo" in verbose, verbose
        edit(f"{demo}/src/main.rs", 9)
        sh("cargo", "build", "-q", cwd=demo)
        assert sh(f"{target}/debug/demo", cwd=demo).stdout.strip()
        r = sh("cargo", "test", "-q", cwd=demo, env={"BOOM": "1", "RUST_BACKTRACE": "1"}, check=False)
        assert re.search(r"demo::tests::inner\n +at \./src/main\.rs:\d+", r.stdout + r.stderr), r.stdout + r.stderr
        print(f"ok: {len(before)} objects, {len(before) - len(after)} swept, {len(after)} kept")


main()
