#!/usr/bin/env -S python3 -I
"""Delete the loose rustc object files (*.rcgu.o) that no binary uses (LIO-676).

Usage: scripts/sweep-rcgu.py [--delete] [--worktrees REPO]... [--in DIR]... [TARGET_DIR]...

Without --delete it only reports what it would delete. --worktrees REPO adds
the target/ of every checkout (main and worktrees) of the git repository REPO;
--in DIR the target/ of every folder directly in DIR (the T3 worktrees of a
repository, say). A REPO that can't be read is reported and skipped.

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
working, but its backtraces lose file:line once its objects are swept. When
nm can't read a binary (Xcode moved, its licence not accepted) nothing is
swept: the run stops with an error.

A target folder is left alone while it is in use: when a process has a file
open below it (a running test binary; Cargo's lock files are asked
with flock instead), or when a build tool (cargo, rustc,
clippy, a linker, dsymutil, a `lockf` waiting for the shared heavy-tests lock)
runs with its working folder in that checkout. A profile folder (debug,
release, <triple>/<profile>) is left alone when Cargo's build-folder locks
can't all be taken at once; with --delete they are held while its files are
deleted, so a build that starts meanwhile waits for them. A build that starts
and finishes while the sweep reads the binaries loses nothing: under the lock,
every binary written since the sweep began is read again.

The Mac runs it hourly from a launchd job (scripts/install-sweep-rcgu.sh):
~/Library/LaunchAgents/local.sweep-rcgu.plist runs ~/.local/bin/sweep-rcgu (a
copy of this file) with --delete --worktrees <main checkout> --in
~/.t3/worktrees/axis-cam-desktop, logging to ~/Library/Logs/sweep-rcgu.log.
macOS doesn't let a launchd job read ~/Documents, where the main checkout is,
so there it reaches only the T3 worktrees; the main checkout's target/ is swept
when it is run by hand.
"""

import fcntl
import os
import re
import shlex
import subprocess
import sys
import time

LOCKS = (".cargo-build-lock", ".cargo-lock", ".cargo-artifact-lock")
MACHO = (b"\xcf\xfa\xed\xfe", b"\xca\xfe\xba\xbe")  # 64-bit Mach-O, universal
NOT_MACHO = (".o", ".rlib", ".rmeta", ".d", ".a", ".json", ".txt", ".rs")
BUILD_TOOLS = re.compile(
    r"^(cargo|cargo-clippy|clippy-driver|rustc|rustdoc|ld|ld64|ld-prime|ld64\.lld|cc|clang|dsymutil|lockf)$")
OSO = re.compile(r" OSO (.+)$")
UNIT_SUBDIRS = ("deps", "build", "examples")


def git_checkouts(repo):
    run = subprocess.run(["git", "-C", repo, "worktree", "list", "--porcelain"], capture_output=True, text=True)
    if run.returncode != 0:
        print(f"can't list the checkouts of {repo}, skipped: {run.stderr.strip()}")
        return []
    return [line[len("worktree "):] for line in run.stdout.splitlines() if line.startswith("worktree ")]


def folders_in(parent):
    try:
        return [e.path for e in os.scandir(parent) if e.is_dir(follow_symlinks=False)]
    except OSError as e:
        print(f"can't list {parent}, skipped: {e}")
        return []


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


def take_locks(prof, create):
    """Cargo's locks on this profile folder, all of them or none. With `create`,
    a missing lock file is made (empty, as Cargo makes it), so a build that
    starts meanwhile waits for it too; a dry run makes none."""
    fds = []
    for name in LOCKS:
        path = os.path.join(prof, name)
        try:
            fd = os.open(path, (os.O_RDWR | os.O_CREAT if create else os.O_RDONLY) | os.O_NOFOLLOW, 0o644)
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
    """The Mach-O files at the top of a profile folder and in its unit folders,
    each file once (Cargo hard-links debug/axis-cam to its copy in deps/)."""
    seen = set()
    for sub in ("",) + UNIT_SUBDIRS:
        for dp, dns, fns in os.walk(os.path.join(prof, sub)):
            if sub == "":
                dns[:] = []
            dns[:] = [d for d in dns if not d.endswith(".dSYM")]
            for fn in fns:
                p = os.path.join(dp, fn)
                if fn.endswith(NOT_MACHO) or os.path.islink(p):
                    continue
                try:
                    st = os.stat(p)
                except OSError:
                    continue
                if (st.st_dev, st.st_ino) not in seen and is_macho(p):
                    seen.add((st.st_dev, st.st_ino))
                    yield p


class NmFailed(Exception):
    pass


_folders = {}


def name_key(path):
    """A file's name in its folder, the folder told by its device and inode: the
    same for any spelling of the path (letter case, symlinks, /tmp or
    /private/tmp), and different for each hard-linked name of one file."""
    folder = os.path.dirname(path)
    if folder not in _folders:
        try:
            st = os.stat(folder)
            _folders[folder] = (st.st_dev, st.st_ino)
        except OSError:
            _folders[folder] = None
    return _folders[folder] and (_folders[folder], os.path.basename(path))


def referenced(bins):
    """The names (name_key) of the loose object files these binaries point at.
    Raises NmFailed when nm can't read one: without its references every
    object would look unused."""
    keep = set()
    for p in bins:
        run = subprocess.run(["nm", "-ap", p], capture_output=True, text=True)
        if run.returncode != 0:
            if not os.path.exists(p):  # removed since it was listed (cargo clean, a rebuild)
                continue
            raise NmFailed(f"nm failed on {p} (exit {run.returncode}): {run.stderr.strip()[:300]}")
        for line in run.stdout.splitlines():
            m = OSO.search(line)
            if m and not m.group(1).endswith(")"):  # `lib….rlib(member.o)` is not a loose file
                key = name_key(m.group(1))
                if key:
                    keep.add(key)
    return keep


def written_since(prof, start):
    """The binaries of a profile folder written (or hard-linked) since `start`."""
    for p in binaries(prof):
        try:
            st = os.stat(p)
        except OSError:
            continue
        if max(st.st_mtime, st.st_ctime) >= start:
            yield p


def object_files(prof):
    for sub in UNIT_SUBDIRS:
        for dp, _, fns in os.walk(os.path.join(prof, sub)):
            for fn in fns:
                if fn.endswith(".rcgu.o"):
                    yield os.path.join(dp, fn)


def main():
    args = sys.argv[1:]
    if not args:
        sys.exit(__doc__)
    delete = False
    targets = []
    while args:
        a = args.pop(0)
        if a == "--delete":
            delete = True
        elif a == "--worktrees" and args:
            targets += [os.path.join(c, "target") for c in git_checkouts(os.path.expanduser(args.pop(0)))]
        elif a == "--in" and args:
            targets += [os.path.join(c, "target") for c in folders_in(os.path.expanduser(args.pop(0)))]
        elif a.startswith("-"):
            sys.exit(__doc__)
        else:
            targets.append(a)
    targets = list(dict.fromkeys(os.path.realpath(os.path.expanduser(t)) for t in targets))
    targets = [t for t in targets if os.path.isdir(t)]
    if not targets:
        print("no target folder found")
        return
    verb = "deleted" if delete else "would delete"

    # References from every binary of every given checkout: clone-cargo-cache's
    # clones of a build script or proc macro point at the checkout they came from.
    # A build can start and finish while the scan runs: what it wrote is newer
    # than `start` (a second of slack for clocks), and is kept below.
    start = time.time() - 1
    profs = {t: list(profile_dirs(t)) for t in targets}
    try:
        keep = referenced(b for ps in profs.values() for prof in ps for b in binaries(prof))
    except NmFailed as e:
        sys.exit(f"{e}\nnothing is swept: without it no object can be told unused")
    hook = os.environ.get("SWEEP_RCGU_AFTER_SCAN")  # scripts/test-sweep-rcgu.py builds here
    if hook:
        subprocess.run(shlex.split(hook), check=True)
    files, tool_cwds = in_use()

    total_files = total_bytes = 0
    for target in targets:
        checkout = os.path.dirname(target)
        if any(under(f, target) for f in files):
            print(f"busy (a file below it is open), left alone: {target}")
            continue
        if any(under(c, checkout) for c in tool_cwds):
            print(f"busy (a build tool runs in the checkout), left alone: {target}")
            continue
        for prof in profs[target]:
            fds = take_locks(prof, create=delete)
            if fds is None:
                print(f"busy (Cargo holds its lock), left alone: {prof}")
                continue
            if not delete:  # a dry run lets go of them at once
                for fd in fds:
                    os.close(fd)
                fds = []
            try:
                # Under the lock nothing more is written here: what a build linked
                # since the scan began is read now.
                try:
                    # A folder made again since the scan (cargo clean, then a build)
                    # has a new inode: forget what the scan learnt of folders.
                    _folders.clear()
                    here = keep | referenced(written_since(prof, start))
                except NmFailed as e:
                    sys.exit(f"{e}\nstopped: nothing more is swept")
                n = kept = 0
                files_here = {}  # (dev, inode): [names it had, names deleted, bytes]
                for p in object_files(prof):
                    try:
                        st = os.lstat(p)
                    except OSError:
                        continue
                    # Not ctime as well: unlinking one name of a file changes the
                    # ctime of its other names, and a reused object has several.
                    if name_key(p) in here:
                        kept += 1
                        continue
                    try:
                        if delete:
                            os.unlink(p)
                    except OSError:
                        continue
                    n += 1
                    f = files_here.setdefault((st.st_dev, st.st_ino), [st.st_nlink, 0, st.st_blocks * 512])
                    f[1] += 1
                # Only a file left with no name frees its blocks: most old names are
                # hard links of a file incremental/ or a newer name still has.
                size = sum(b for links, gone, b in files_here.values() if gone >= links)
                if n:
                    print(f"{verb} {n} ({size / 2**30:.2f} GB freed), kept {kept} in use: {prof}")
                total_files += n
                total_bytes += size
            finally:
                for fd in fds:
                    os.close(fd)
    print(f"{verb} {total_files} files, freeing {total_bytes / 2**30:.1f} GB (less where a file"
          " shares blocks with an APFS clone)")


main()
