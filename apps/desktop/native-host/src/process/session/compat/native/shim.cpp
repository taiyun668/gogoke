// Fixed Codex 0.149.0 x64 LPAC path compatibility. No process-wide code detour.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <winnt.h>
#include "detours.h"

namespace {
constexpr DWORD kMaxPathUnits = 32768;
constexpr wchar_t kNtEnv[] = L"GOGOKE_LPAC_PATH_NT_ROOT";
constexpr wchar_t kDosEnv[] = L"GOGOKE_LPAC_PATH_DOS_ROOT";
using FinalPath = DWORD (WINAPI *)(HANDLE, LPWSTR, DWORD, DWORD);
FinalPath g_original = nullptr;

bool ascii_exact(const char* a, const char* b, size_t bound) {
    for (size_t i = 0; i < bound; ++i) {
        if (a[i] != b[i]) return false;
        if (!a[i]) return true;
    }
    return false;
}

bool ascii_name(const char* a, size_t bound, const char* wanted) {
    size_t i = 0;
    for (; i < bound; ++i) {
        char x = a[i];
        char y = wanted[i];
        if (x >= 'A' && x <= 'Z') x = static_cast<char>(x + 32);
        if (y >= 'A' && y <= 'Z') y = static_cast<char>(y + 32);
        if (x != y) return false;
        if (!x) return true;
    }
    return false;
}

bool range_ok(DWORD rva, size_t bytes, DWORD image_size) {
    return rva && rva < image_size && bytes <= image_size - rva;
}

bool within_root(const wchar_t* path, DWORD path_len,
                 const wchar_t* root, DWORD root_len) {
    if (!root_len || path_len < root_len) return false;
    if (CompareStringOrdinal(path, root_len, root, root_len, TRUE) != CSTR_EQUAL)
        return false;
    return path_len == root_len || path[root_len] == L'\\';
}

bool read_mapping(wchar_t (&nt)[kMaxPathUnits], DWORD* nt_len,
                  wchar_t (&dos)[kMaxPathUnits], DWORD* dos_len) {
    *nt_len = GetEnvironmentVariableW(kNtEnv, nt, kMaxPathUnits);
    *dos_len = GetEnvironmentVariableW(kDosEnv, dos, kMaxPathUnits);
    if (!*nt_len || !*dos_len || *nt_len >= kMaxPathUnits ||
        *dos_len >= kMaxPathUnits) return false;
    // The host supplies the exact handle-derived F home; environment data
    // selects a spelling only. It does not authorize opening the target.
    if (*nt_len < 9 || nt[0] != L'\\' ||
        CompareStringOrdinal(nt, 8, L"\\Device\\", 8, TRUE) != CSTR_EQUAL)
        return false;
    if (*dos_len < 7 || dos[0] != L'\\' || dos[1] != L'\\' ||
        dos[2] != L'?' || dos[3] != L'\\' ||
        !((dos[4] >= L'A' && dos[4] <= L'Z') ||
          (dos[4] >= L'a' && dos[4] <= L'z')) ||
        dos[5] != L':' || dos[6] != L'\\') return false;
    if (nt[*nt_len - 1] == L'\\' || dos[*dos_len - 1] == L'\\')
        return false;
    return true;
}

DWORD WINAPI compatible_final_path(HANDLE file, LPWSTR output,
                                   DWORD capacity, DWORD flags) {
    const DWORD result = g_original(file, output, capacity, flags);
    const DWORD original_error = GetLastError();
    if (flags != 0 || result != 0 || original_error != ERROR_ACCESS_DENIED)
        return SetLastError(original_error), result;

    // Fixed stack storage avoids heap activity and caps every Win32 length.
    wchar_t nt_root[kMaxPathUnits], dos_root[kMaxPathUnits];
    DWORD nt_root_len = 0, dos_root_len = 0;
    if (!read_mapping(nt_root, &nt_root_len, dos_root, &dos_root_len))
        return SetLastError(original_error), result;
    wchar_t actual_nt[kMaxPathUnits];
    const DWORD actual_len = g_original(file, actual_nt, kMaxPathUnits,
                                        VOLUME_NAME_NT);
    if (!actual_len || actual_len >= kMaxPathUnits ||
        !within_root(actual_nt, actual_len, nt_root, nt_root_len))
        return SetLastError(original_error), result;
    const DWORD suffix_len = actual_len - nt_root_len;
    if (dos_root_len > kMaxPathUnits - 1 - suffix_len)
        return SetLastError(original_error), result;
    const DWORD translated_len = dos_root_len + suffix_len;
    // Win32 returns required size INCLUDING NUL for a short or zero buffer;
    // on success it returns the copied length EXCLUDING NUL.
    if (capacity <= translated_len) {
        SetLastError(ERROR_INSUFFICIENT_BUFFER);
        return translated_len + 1;
    }
    if (!output) return SetLastError(original_error), result;
    for (DWORD i = 0; i < dos_root_len; ++i) output[i] = dos_root[i];
    for (DWORD i = 0; i < suffix_len; ++i)
        output[dos_root_len + i] = actual_nt[nt_root_len + i];
    output[translated_len] = L'\0';
    SetLastError(ERROR_SUCCESS);
    return translated_len;
}

// The exact installed Codex 0.149.0 x64 image imports this name once from
// kernel32.dll. Root independently pins its bytes before suspended launch.
// Kept separate from the write so cloud tests exercise this same PE parser.
void** exact_main_import_slot(BYTE* image) {
    if (!image) return nullptr;
    auto* dos = reinterpret_cast<IMAGE_DOS_HEADER*>(image);
    if (dos->e_magic != IMAGE_DOS_SIGNATURE || dos->e_lfanew <= 0 ||
        dos->e_lfanew > 4096) return nullptr;
    auto* pe = reinterpret_cast<IMAGE_NT_HEADERS64*>(image + dos->e_lfanew);
    if (pe->Signature != IMAGE_NT_SIGNATURE ||
        pe->FileHeader.Machine != IMAGE_FILE_MACHINE_AMD64 ||
        pe->OptionalHeader.Magic != IMAGE_NT_OPTIONAL_HDR64_MAGIC ||
        pe->OptionalHeader.NumberOfRvaAndSizes <= IMAGE_DIRECTORY_ENTRY_IMPORT)
        return nullptr;
    const DWORD image_size = pe->OptionalHeader.SizeOfImage;
    const auto imports = pe->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT];
    if (!range_ok(imports.VirtualAddress, imports.Size, image_size) ||
        imports.Size < sizeof(IMAGE_IMPORT_DESCRIPTOR)) return nullptr;
    auto* descriptors = reinterpret_cast<IMAGE_IMPORT_DESCRIPTOR*>(
        image + imports.VirtualAddress);
    const size_t count = imports.Size / sizeof(IMAGE_IMPORT_DESCRIPTOR);
    if (count > 64) return nullptr;
    void** slot = nullptr;
    bool descriptor_end = false;
    for (size_t d = 0; d < count; ++d) {
        const auto& desc = descriptors[d];
        if (!desc.Name && !desc.FirstThunk && !desc.OriginalFirstThunk &&
            !desc.TimeDateStamp && !desc.ForwarderChain) {
            descriptor_end = true;
            break;
        }
        if (!range_ok(desc.Name, 13, image_size) ||
            !range_ok(desc.OriginalFirstThunk, sizeof(IMAGE_THUNK_DATA64), image_size) ||
            !range_ok(desc.FirstThunk, sizeof(IMAGE_THUNK_DATA64), image_size)) return nullptr;
        const char* dll = reinterpret_cast<const char*>(image + desc.Name);
        const bool kernel32 = ascii_name(dll, image_size - desc.Name, "kernel32.dll");
        auto* names = reinterpret_cast<IMAGE_THUNK_DATA64*>(image + desc.OriginalFirstThunk);
        auto* values = reinterpret_cast<IMAGE_THUNK_DATA64*>(image + desc.FirstThunk);
        const size_t max_names = (image_size - desc.OriginalFirstThunk) / sizeof(IMAGE_THUNK_DATA64);
        const size_t max_values = (image_size - desc.FirstThunk) / sizeof(IMAGE_THUNK_DATA64);
        const size_t available = max_names < max_values ? max_names : max_values;
        const size_t max = available < 1024 ? available : 1024;
        bool ended = false;
        for (size_t i = 0; i < max; ++i) {
            if (!names[i].u1.AddressOfData) { ended = true; break; }
            if (IMAGE_SNAP_BY_ORDINAL64(names[i].u1.Ordinal)) continue;
            const DWORD rva = static_cast<DWORD>(names[i].u1.AddressOfData);
            if (names[i].u1.AddressOfData != rva ||
                !range_ok(rva, sizeof(WORD) + 1, image_size)) return nullptr;
            const char* name = reinterpret_cast<const char*>(image + rva + sizeof(WORD));
            if (ascii_exact(name, "GetFinalPathNameByHandleW", image_size - rva - sizeof(WORD))) {
                if (!kernel32) return nullptr;
                if (slot) return nullptr;
                slot = reinterpret_cast<void**>(&values[i].u1.Function);
            }
        }
        if (!ended) return nullptr;
    }
    return descriptor_end && slot && *slot ? slot : nullptr;
}

bool patch_exact_main_import() {
    auto* image = static_cast<BYTE*>(static_cast<void*>(GetModuleHandleW(nullptr)));
    void** slot = exact_main_import_slot(image);
    if (!slot) return false;
    auto original = reinterpret_cast<FinalPath>(*slot);
    DWORD old_protection = 0;
    if (!VirtualProtect(slot, sizeof(void*), PAGE_READWRITE, &old_protection)) return false;
    const void* observed = InterlockedCompareExchangePointer(
        reinterpret_cast<PVOID volatile*>(slot),
        reinterpret_cast<void*>(&compatible_final_path),
        reinterpret_cast<void*>(original));
    DWORD ignored = 0;
    const BOOL restored = VirtualProtect(slot, sizeof(void*), old_protection, &ignored);
    if (observed != reinterpret_cast<void*>(original) || !restored) return false;
    g_original = original;
    return true;
}
} // namespace

#ifndef GOGOKE_LPAC_PATH_TEST
// Ordinal 1 is required by the Detours import edit. This package never takes
// the cross-bitness helper route and deliberately provides no helper behavior.
extern "C" void CALLBACK DetourFinishHelperProcess(HWND, HINSTANCE, LPSTR, INT) {
    SetLastError(ERROR_NOT_SUPPORTED);
}

extern "C" BOOL WINAPI DllMain(HINSTANCE module, DWORD reason, LPVOID) {
    if (reason != DLL_PROCESS_ATTACH) return TRUE;
    DisableThreadLibraryCalls(module);
    // Detours' import edit must be restored before reading the target IAT.
    // No LoadLibrary, thread creation, or waits occur under loader lock.
    if (!DetourRestoreAfterWith()) return FALSE;
    return patch_exact_main_import() ? TRUE : FALSE;
}
#endif
