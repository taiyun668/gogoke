"""Closed installed physical view with an explicit component transition binding."""
import ctypes
import hashlib
import json
import os
import re
import sys
import winreg
from ctypes import wintypes
from datetime import datetime, timezone
from pathlib import Path

if sys.flags.optimize != 0:
    raise RuntimeError('Optimized Python cannot run evidence validation')

REPARSE = 0x400


def require(ok, reason):
    if not ok:
        raise RuntimeError(reason)


def regular_binding_path(path):
    for part in (path, *path.parents):
        if part == part.parent:
            break
        require(not part.lstat().st_file_attributes & REPARSE, 'reparse binding path')


def component(value):
    require(isinstance(value, dict) and set(value) == {'candidateNumber', 'sourceCommit', 'rootLeaf'},
            'exact component tuple required')
    number, source, leaf = value['candidateNumber'], value['sourceCommit'], value['rootLeaf']
    require(type(number) is int and number > 0 and isinstance(source, str) and
            re.fullmatch(r'[a-f0-9]{40}', source) and isinstance(leaf, str) and
            leaf == f'gogoke37-m2-{number}-{source[:8]}', 'component root leaf/source mismatch')
    return value


def binding(input_file, output_file):
    source, out = Path(input_file), Path(output_file)
    require(source.is_absolute() and out.is_absolute() and
            source.drive.upper() == 'D:' and out.drive.upper() == 'D:' and
            '..' not in source.parts and '..' not in out.parts and
            source.parent == out.parent and source != out and
            source.suffix == '.json' and out.suffix == '.json',
            'explicit same-directory D JSON binding/output required')
    regular_binding_path(source)
    require(source.is_file(), 'regular binding file required')
    try:
        out.lstat()
    except FileNotFoundError:
        pass
    else:
        raise RuntimeError('output must be fresh')
    cfg = json.loads(source.read_text(encoding='utf-8-sig'))
    require(isinstance(cfg, dict) and set(cfg) == {'schema', 'state', 'executionEnabled',
            'acceptance', 'toolSourceSha256', 'oldComponent', 'newComponent'} and
            cfg['schema'] == 'gogoke.37.installed-physical-view-input.v1' and
            cfg['state'] == 'ROOT_BOUND_COMPONENT_TRANSITION' and
            cfg['executionEnabled'] is True and cfg['acceptance'] is False,
            'bound non-acceptance physical view input required')
    old, new = component(cfg['oldComponent']), component(cfg['newComponent'])
    require(new['candidateNumber'] == old['candidateNumber'] + 1 and
            re.fullmatch(r'[a-f0-9]{64}', cfg['toolSourceSha256']) and
            hashlib.sha256(Path(__file__).read_bytes()).hexdigest() == cfg['toolSourceSha256'],
            'component transition or loaded physical tool source differs')
    return old['rootLeaf']


class Id(ctypes.Structure):
    _fields_=[('volume',ctypes.c_ulonglong),('file',ctypes.c_ubyte*16)]
class Tag(ctypes.Structure):
    _fields_=[('attributes',wintypes.DWORD),('tag',wintypes.DWORD)]


def obj(path):
    s=path.lstat();assert not s.st_file_attributes&0x400,'reparse source'
    h=k.CreateFileW(str(path),0x80,7,None,3,0x02200000,None)
    if h in (None,ctypes.c_void_p(-1).value):raise ctypes.WinError(ctypes.get_last_error())
    try:
        t=Tag();i=Id();buf=ctypes.create_unicode_buffer(32768)
        if not k.GetFileInformationByHandleEx(h,9,ctypes.byref(t),ctypes.sizeof(t)):raise ctypes.WinError(ctypes.get_last_error())
        assert not t.attributes&0x400,'handle reparse source'
        if not k.GetFileInformationByHandleEx(h,18,ctypes.byref(i),ctypes.sizeof(i)):raise ctypes.WinError(ctypes.get_last_error())
        n=k.GetFinalPathNameByHandleW(h,buf,len(buf),0)
        if not n or n>=len(buf):raise ctypes.WinError(ctypes.get_last_error())
        actual=buf.value.removeprefix('\\\\?\\')
        assert os.path.normcase(actual)==os.path.normcase(str(path.absolute())), 'SCOPE_MISMATCH leaf/root finalPath: '+str(path)
        return {'path':str(path),'kind':'directory' if t.attributes&0x10 else 'file',
          'finalPath':buf.value,'volume':f'{i.volume:016x}','fileId':bytes(i.file).hex()}
    finally:
        if not k.CloseHandle(h):raise ctypes.WinError(ctypes.get_last_error())


def capture(out, old_root_leaf):
    global k
    k=ctypes.WinDLL('kernel32',use_last_error=True)
    k.CreateFileW.argtypes=[wintypes.LPCWSTR,wintypes.DWORD,wintypes.DWORD,ctypes.c_void_p,wintypes.DWORD,wintypes.DWORD,wintypes.HANDLE]
    k.CreateFileW.restype=wintypes.HANDLE
    k.GetFinalPathNameByHandleW.argtypes=[wintypes.HANDLE,wintypes.LPWSTR,wintypes.DWORD,wintypes.DWORD]
    k.GetFinalPathNameByHandleW.restype=wintypes.DWORD
    k.GetFileInformationByHandleEx.argtypes=[wintypes.HANDLE,ctypes.c_int,ctypes.c_void_p,wintypes.DWORD]
    k.GetFileInformationByHandleEx.restype=wintypes.BOOL
    k.CloseHandle.argtypes=[wintypes.HANDLE]
    home=Path(os.environ['USERPROFILE'])
    local=home/'AppData/Local';roaming=home/'AppData/Roaming'
    assert Path(os.environ['LOCALAPPDATA'])==local and Path(os.environ['APPDATA'])==roaming
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER,r'Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke') as key:
        registered=Path(winreg.QueryValueEx(key,'InstallLocation')[0])
    roots={}
    for name,path in [('formal',local/'Programs/gogoke'),('registeredFormal',registered),
                      ('formalData',roaming/'app.gogoke.desktop')]:
        # All descendants, hidden included, no reparse traversal.
        records=[]
        stack=[path]
        while stack:
            p=stack.pop();record=obj(p);records.append(record)
            if record['kind']=='directory':
                stack.extend(sorted(p.iterdir(),key=lambda q:str(q).casefold(),reverse=True))
        roots[name]=records
    data=roaming/'app.gogoke.desktop.candidate/product-authority'
    roots['candidateData']=[obj(data),obj(data/'state.sqlite')]
    old_root=local/old_root_leaf
    try:
        old_stat=old_root.lstat()
    except FileNotFoundError:
        old_stat=None
    if old_stat is None:
        old_candidate={'state':'ABSENT','path':str(old_root)}
    else:
        assert not old_stat.st_file_attributes&0x400,'old candidate root is reparse'
        old_handle=obj(old_root)
        assert old_handle['kind']=='directory','old candidate root is not a directory'
        entries=list(old_root.iterdir())
        if entries:
            core=[obj(old_root/name) for name in ('gogoke.exe','gogoke-native-host.exe','resource-index.json')]
            old_candidate={'state':'PRESENT','path':str(old_root),'root':old_handle,
                           'core':core,'entryCount':len(entries)}
        else:
            old_candidate={'state':'EMPTY','path':str(old_root),'root':old_handle,'entryCount':0}
    result={'state':'REAL_ORDINARY_LEAF_PATHS_ONLY','recordedUtc':datetime.now(timezone.utc).isoformat(),
     'roots':roots,'oldCandidateRoot':old_candidate,'databaseOpened':False,'contentsRead':False,
     'fileWrites':'D evidence only','acceptance':False}
    with out.open('x',encoding='utf-8') as f:json.dump(result,f,indent=2);f.write('\n')
    print('REAL_ORDINARY_LEAF_PATHS_ONLY_NO_CONTENTS_NO_SQL')


if __name__ == '__main__':
    require(len(sys.argv) == 3, 'output and explicit physical input required')
    leaf = binding(sys.argv[2], sys.argv[1])
    capture(Path(sys.argv[1]), leaf)
