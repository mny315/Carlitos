#!/usr/bin/env python3
"""Check the release PE imports without requiring dumpbin or third-party modules."""
import hashlib
import pathlib
import struct
import sys


def inspect(path):
    data = path.read_bytes()
    assert data[:2] == b"MZ", "not a Windows executable"
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    assert data[pe:pe + 4] == b"PE\0\0"
    machine, count = struct.unpack_from("<HH", data, pe + 4)
    assert machine == 0x8664, "expected Windows x64"
    optional_size = struct.unpack_from("<H", data, pe + 20)[0]
    optional = pe + 24
    assert struct.unpack_from("<H", data, optional)[0] == 0x20B
    assert struct.unpack_from("<H", data, optional + 68)[0] == 2, "expected GUI subsystem"
    sections = []
    for index in range(count):
        header = optional + optional_size + index * 40
        size, address, raw_size, raw = struct.unpack_from("<IIII", data, header + 8)
        sections.append((address, max(size, raw_size), raw))

    def offset(address):
        for start, size, raw in sections:
            if start <= address < start + size:
                return raw + address - start
        raise AssertionError(f"invalid RVA {address:x}")

    def string(address):
        start = offset(address)
        return data[start:data.index(0, start)].decode("ascii").lower()

    imported = set()
    import_rva = struct.unpack_from("<I", data, optional + 120)[0]
    cursor = offset(import_rva)
    while any(data[cursor:cursor + 20]):
        imported.add(string(struct.unpack_from("<I", data, cursor + 12)[0]))
        cursor += 20
    delay_rva = struct.unpack_from("<I", data, optional + 112 + 13 * 8)[0]
    if delay_rva:
        cursor = offset(delay_rva)
        while any(data[cursor:cursor + 32]):
            imported.add(string(struct.unpack_from("<I", data, cursor + 4)[0]))
            cursor += 32
    system = set("""kernel32.dll ntdll.dll user32.dll gdi32.dll advapi32.dll
        bcrypt.dll bcryptprimitives.dll ole32.dll oleaut32.dll combase.dll
        comctl32.dll shell32.dll shlwapi.dll uxtheme.dll uiautomationcore.dll
        dwrite.dll dwmapi.dll mfplat.dll mfreadwrite.dll propsys.dll opengl32.dll
        imm32.dll xaudio2_9.dll crypt32.dll ws2_32.dll userenv.dll secur32.dll
        version.dll winmm.dll winspool.drv msvcrt.dll ucrtbase.dll""".split())
    external = sorted(name for name in imported if name not in system
                      and not name.startswith(("api-ms-win-", "ext-ms-win-")))
    assert not external, f"non-system dependencies: {external}"
    assert {"mfplat.dll", "mfreadwrite.dll", "xaudio2_9.dll"} <= imported
    print(f"{path}: {len(data):,} bytes; {len(imported)} system imports; no bundled DLLs required")
    print(f"SHA256 {hashlib.sha256(data).hexdigest()}")


if __name__ == "__main__":
    inspect(pathlib.Path(sys.argv[1]))
