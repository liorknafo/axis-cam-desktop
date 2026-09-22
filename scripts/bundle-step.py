#!/usr/bin/env python3
"""Copy the built CAD reader and its non-system dylibs into a relocatable app."""
import json, os, pathlib, shutil, subprocess, sys
def replace_copy(source,dest):
    if dest.exists(): dest.unlink()
    shutil.copy2(source,dest)
    dest.chmod(dest.stat().st_mode | 0o200)
app = pathlib.Path(sys.argv[1]).resolve()
target = pathlib.Path(sys.argv[2])
binary = app / 'Contents/MacOS/axis-step'
frameworks = app / 'Contents/Frameworks'
frameworks.mkdir(parents=True, exist_ok=True)
# Cargo's active build-script output identifies the helper used by this build.
helpers = list((target / 'release/build').glob('axis-cam-*/out/axis-step'))
if not helpers:
    raise SystemExit('No compiled STEP helper; package requires Open CASCADE support.')
helper = max(helpers, key=lambda p: p.stat().st_mtime)
replace_copy(helper, binary)
prefix = pathlib.Path(os.environ.get('OCCT_PREFIX', '/opt/homebrew/opt/opencascade'))
search = [prefix / 'lib', pathlib.Path('/opt/homebrew/lib')]
for resource in ['XSTEPResource', 'XSMessage', 'UnitsAPI']:
    source = prefix / 'share/opencascade/resources' / resource
    if source.exists(): shutil.copytree(source, app / 'Contents/Resources/OCCT' / resource, dirs_exist_ok=True)
def deps(path):
    return [line.strip().split(' (')[0] for line in subprocess.check_output(['otool','-L',str(path)], text=True).splitlines()[1:]]
seen = set()
origins = {}
def bundle(source, dest, executable=False):
    if dest.name in seen:
        return
    seen.add(dest.name)
    for dep in deps(source):
        if dep.startswith(('/System/', '/usr/lib/')):
            continue
        if dep.startswith('@rpath/'):
            choices = [source.parent / dep[7:]] + [p / dep[7:] for p in search]
            resolved = next((p for p in choices if p.exists()), None)
        elif dep.startswith('@loader_path/'):
            resolved = source.parent / dep[len('@loader_path/'):]
        else:
            resolved = pathlib.Path(dep)
        if resolved is None or not resolved.exists():
            raise RuntimeError(f'Cannot resolve {dep} used by {source}')
        if resolved.resolve() == source.resolve():
            continue # dylib install id, not a dependency
        libdest = frameworks / resolved.name
        if resolved.name not in seen:
            replace_copy(resolved, libdest)
            origins[resolved.name] = str(resolved.resolve())
            bundle(resolved, libdest)
        new = ('@executable_path/../Frameworks/' if executable else '@loader_path/') + resolved.name
        subprocess.run(['install_name_tool','-change',dep,new,str(dest)],check=True,capture_output=True)
    if not executable:
        subprocess.run(['install_name_tool','-id','@rpath/'+dest.name,str(dest)],check=True,capture_output=True)
    # Remove build-machine search paths after making every dependency explicit.
    lines = subprocess.check_output(['otool','-l',str(dest)],text=True).splitlines()
    for i,line in enumerate(lines):
        if line.strip() == 'cmd LC_RPATH':
            path = lines[i+2].strip().split(' (')[0].removeprefix('path ')
            if path.startswith('/opt/'):
                subprocess.run(['install_name_tool','-delete_rpath',path,str(dest)],check=True,capture_output=True)
bundle(helper,binary,True)
licenses = app / 'Contents/Resources/ThirdParty'
licenses.mkdir(parents=True,exist_ok=True)
formulae = set()
for lib,path in origins.items():
    p = pathlib.Path(path)
    parts = p.parts
    if 'Cellar' not in parts:
        continue
    i = parts.index('Cellar')
    formula, version = parts[i+1:i+3]
    if (formula,version) in formulae:
        continue
    formulae.add((formula,version))
    root = pathlib.Path(*parts[:i+3])
    out = licenses / f'{formula}-{version}'
    out.mkdir(exist_ok=True)
    candidates = [*root.glob('*LICENSE*'), *root.glob('*COPYING*'), *root.glob('*NOTICE*'), *root.glob('*COPYRIGHT*')]
    if formula == 'opencascade':
        candidates += list((root/'share/doc/opencascade').glob('*LGPL*'))
    if formula == 'freetype':
        candidates += list((root/'share/doc/freetype').rglob('*LICENSE*'))
    for item in candidates:
        if item.is_file(): replace_copy(item,out/item.name)
    receipt=root/'INSTALL_RECEIPT.json'
    if receipt.exists():replace_copy(receipt,out/'HOMEBREW_RECEIPT.json')
shutil.copy2(pathlib.Path(__file__).parent.parent/'assets/licenses/FTL.TXT', licenses/'FTL.TXT')
shutil.copy2(pathlib.Path(__file__).parent.parent/'THIRD_PARTY.md', licenses/'THIRD_PARTY.md')
(licenses/'bundled-libraries.json').write_text(json.dumps(origins,indent=2))
print(f'Bundled STEP reader and {len(origins)} libraries from: '+', '.join(sorted(f'{f} {v}' for f,v in formulae)))
for path in [*frameworks.glob('*.dylib'),binary]:
    subprocess.run(['xattr','-d','com.apple.FinderInfo',str(path)],capture_output=True)
    subprocess.run(['xattr','-d','com.apple.ResourceFork',str(path)],capture_output=True)
    subprocess.run(['codesign','--force','--sign','-',str(path)],check=True,capture_output=True)
    for dep in deps(path):
        if dep.startswith('/opt/'):
            raise RuntimeError(f'Unbundled runtime dependency in {path}: {dep}')
