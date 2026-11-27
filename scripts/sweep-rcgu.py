#!/usr/bin/env -S python3 -I
"""Delete the loose rustc object files (*.rcgu.o) that no binary uses (LIO-676).

Usage: scripts/sweep-rcgu.py [--delete] [--worktrees REPO]... [TARGET_DIR]...

Without --delete it only reports what it would delete. --worktrees REPO adds
the target/ of every checkout (main and worktrees) of the git repository REPO.

Why they pile up: on macOS the dev profile builds with split-debuginfo =
"unpacked". rustc leaves each crate's object files in deps/ (and
build/<pkg>-<hash>/), and a linked binary's debug map (its OSO entries) points
at them for file:line in backtraces and the debugger. Two kinds are never read
again:

* every incremental build of a crate hard-links its codegen units into deps/
  under a new session suffix (`axis_cam-<hash>.<cgu>.<suffix>.rcgu.o`, 256 a
  build for axis-cam and axis-core) and never removes the earlier names, so
  each edit-and-test cycle adds a set and nothing deletes them;
* a library's objects are also packed in its .rlib, and binaries point at the
  copy in the .rlib (`libuc100-<hash>.rlib(uc100-<hash>…rcgu.o)`), not at the
  loose file.

So an object file is kept when a Mach-O file (an executable, a test binary, a
build script, a .dylib such as a proc macro) in any profile folder of the
given target folders names it in its debug map, and deleted otherwise. Cargo's
fingerprints don't look at these files, so no build is redone, the next
incremental build works as before, and every backtrace keeps its file:line.
A binary copied out of target/ (into a thread's evidence folder, say) keeps
working, but its backtraces lose file:line once its objects are swept.

A target folder is left alone while it is in use: when a process has a file
open below it (a running test binary; Cargo's lock files are asked
with flock instead), or when a build tool (cargo, rustc,
clippy, a linker, dsymutil, a `lockf` waiting for the shared heavy-tests lock)
runs with its working folder in that checkout. A profile folder (debug,
release, <triple>/<profile>) is left alone when Cargo's build-folder locks
can't all be taken at once; with --delete they are held while its files are
deleted, so a build that starts meanwhile waits for them.

The Mac runs it hourly over every axis-cam-desktop checkout from a launchd job:
~/Library/LaunchAgents/local.sweep-rcgu.plist runs ~/.local/bin/sweep-rcgu (a
copy of this file) with --delete --worktrees <main checkout>, logging to
~/Library/Logs/sweep-rcgu.log.
"""

import fcntl
import os
import re
import subprocess
import sys

LOCKS = (".cargo-build-lock", ".cargo-lock", ".cargo-artifact-lock")
MACHO = (b"\xcf\xfa\xed\xfe", b"\xca\xfe\xba\xbe")  # 64-bit Mach-O, universal
NOT_MACHO = (".o", ".rlib", ".rmeta", ".d", ".a", ".json", ".txt", ".rs")
BUILD_TOOLS = re.compile(
    r"^(cargo|cargo-clippy|clippy-driver|rustc|rustdoc|ld|ld64|ld-prime|ld64\.lld|cc|clang|dsymutil|lockf)$")
OSO = re.compile(r" OSO (.+)$")
UNIT_SUBDIRS = ("deps", "build", "examples")


def git_checkouts(repo):
    out = subprocess.run(["git", "-C", repo, "worktree", "list", "--porcelain"],
                         check=True, capture_output=True, text=True).stdout
    return [line[len("worktree "):] for line in out.splitlines() if line.startswith("worktree ")]


def in_use():
    """Every file any process has open, and every build tool's working folder."""
    run = subprocess.run(["lsof", "-n", "-P", "+c", "0", "-F", "pcfn"], capture_output=True, text=True)
    if not run.stdout:
        sys.exit(f"lsof gave nothing (exit {run.returncode}): can't tell what is in use, so nothing is swept")
    files, tool_cwds = [], []
    cmd = fd = None
    for line in run.stdout.splitlines():
        tag, val = line[:1], line[1:]
        if tag == "p":
            cmd = fd = None
        elif tag == "c":
            cmd = val
        elif tag == "f":
            fd = val
        elif tag == "n":
            if fd == "cwd":
                if cmd and BUILD_TOOLS.match(cmd):
                    tool_cwds.append(os.path.realpath(val))
            elif os.path.basename(val) not in LOCKS:  # Cargo's locks are asked with flock below
                files.append(val)
    return files, tool_cwds


def under(path, root):
    return path == root or path.startswith(root.rstrip("/") + "/")


def profile_dirs(target):
    """Folders holding a .fingerprint/: debug, release, <triple>/<profile>, nested target folders."""
    for dp, dns, _ in os.walk(target):
        if ".fingerprint" in dns:
            yield dp
            dns[:] = []  # a profile folder holds no other profile folder
        else:
            dns[:] = sorted(d for d in dns if d not in ("incremental", "doc", "tmp") and not d.endswith(".dSYM"))


def take_locks(prof):
    """Cargo's locks on this profile folder, all of them or none."""
    fds = []
    for name in LOCKS:
        path = os.path.join(prof, name)
        try:
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        except FileNotFoundError:
            continue
        except OSError:
            break
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            os.close(fd)
            break
        fds.append(fd)
    else:
        return fds
    for fd in fds:
        os.close(fd)
    return None


def is_macho(path):
    try:
        with open(path, "rb") as f:
            return f.read(4) in MACHO
    except OSError:
        return False


def binaries(prof):
    """The Mach-O files at the top of a profile folder and in its unit folders."""
    for sub in ("",) + UNIT_SUBDIRS:
        for dp, dns, fns in os.walk(os.path.join(prof, sub)):
            if sub == "":
                dns[:] = []
            dns[:] = [d for d in dns if not d.endswith(".dSYM")]
            for fn in fns:
                p = os.path.join(dp, fn)
                if not fn.endswith(NOT_MACHO) and not os.path.islink(p) and is_macho(p):
                    yield p


def referenced(profs):
    """The real paths of the loose object files the binaries in these folders point at."""
    keep = set()
    for prof in profs:
        for p in binaries(prof):
            out = subprocess.run(["nm", "-ap", p], capture_output=True, text=True).stdout
            for line in out.splitlines():
                m = OSO.search(line)
                if m and not m.group(1).endswith(")"):  # `lib….rlib(member.o)` is not a loose file
                    keep.add(os.path.realpath(m.group(1)))
    return keep


def object_files(prof):
    for sub in UNIT_SUBDIRS:
        for dp, _, fns in os.walk(os.path.join(prof, sub)):
            for fn in fns:
                if fn.endswith(".rcgu.o"):
                    yield os.path.join(dp, fn)


def main():
    args = sys.argv[1:]
    delete = False
    targets = []
    while args:
        a = args.pop(0)
        if a == "--delete":
            delete = True
        elif a == "--worktrees" and args:
            targets += [os.path.join(c, "target") for c in git_checkouts(os.path.expanduser(args.pop(0)))]
        elif a.startswith("-"):
            sys.exit(__doc__)
        else:
            targets.append(a)
    targets = list(dict.fromkeys(os.path.realpath(os.path.expanduser(t)) for t in targets))
    targets = [t for t in targets if os.path.isdir(t)]
    if not targets:
        sys.exit(__doc__)
    verb = "deleted" if delete else "would delete"

    # References from every binary of every given checkout: clone-cargo-cache's
    # clones of a build script or proc macro point at the checkout they came from.
    profs = {t: list(profile_dirs(t)) for t in targets}
    keep = referenced(p for ps in profs.values() for p in ps)
    files, tool_cwds = in_use()

    total_files = total_bytes = 0
    seen = set()
    for target in targets:
        checkout = os.path.dirname(target)
        if any(under(f, target) for f in files):
            print(f"busy (a file below it is open), left alone: {target}")
            continue
        if any(under(c, checkout) for c in tool_cwds):
            print(f"busy (a build tool runs in the checkout), left alone: {target}")
            continue
        for prof in profs[target]:
            fds = take_locks(prof)
            if fds is None:
                print(f"busy (Cargo holds its lock), left alone: {prof}")
                continue
            if not delete:  # a dry run never makes a build wait
                for fd in fds:
                    os.close(fd)
                fds = []
            try:
                n = size = kept = 0
                for p in object_files(prof):
                    if os.path.realpath(p) in keep:
                        kept += 1
                        continue
                    try:
                        st = os.lstat(p)
                        if delete:
                            os.unlink(p)
                    except OSError:
                        continue
                    n += 1
                    if (st.st_dev, st.st_ino) not in seen:  # hard links: count a file once
                        seen.add((st.st_dev, st.st_ino))
                        size += st.st_blocks * 512
                if n:
                    print(f"{verb} {n} ({size / 2**30:.2f} GB), kept {kept} in use: {prof}")
                total_files += n
                total_bytes += size
            finally:
                for fd in fds:
                    os.close(fd)
    print(f"{verb} {total_files} files, at most {total_bytes / 2**30:.1f} GB (less where a file is"
          " still hard-linked from incremental/ or shares blocks with an APFS clone)")


main()
