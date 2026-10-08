// Fixed x64 LPAC main-image IAT compatibility and Claude pipe failure observation.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <winnt.h>
#include "detours.h"

namespace {
constexpr DWORD kMaxPathUnits = 32768;
constexpr wchar_t kNtEnv[] = L"GOGOKE_LPAC_PATH_NT_ROOT";
constexpr wchar_t kDosEnv[] = L"GOGOKE_LPAC_PATH_DOS_ROOT";
constexpr wchar_t kCountEnv[] = L"GOGOKE_LPAC_PATH_ROOT_COUNT";
constexpr wchar_t kNtEnv1[] = L"GOGOKE_LPAC_PATH_NT_ROOT_1";
constexpr wchar_t kDosEnv1[] = L"GOGOKE_LPAC_PATH_DOS_ROOT_1";
constexpr wchar_t kNtEnv2[] = L"GOGOKE_LPAC_PATH_NT_ROOT_2";
constexpr wchar_t kDosEnv2[] = L"GOGOKE_LPAC_PATH_DOS_ROOT_2";
constexpr wchar_t kModeEnv[] = L"GOGOKE_LPAC_COMPAT_MODE";
constexpr wchar_t kClaudeMode[] = L"CLAUDE_PIPE_V1";
constexpr DWORD kMaxRoots = 3;
using FinalPath = DWORD (WINAPI *)(HANDLE, LPWSTR, DWORD, DWORD);
FinalPath g_original = nullptr;
using NamedPipeA = HANDLE (WINAPI *)(LPCSTR, DWORD, DWORD, DWORD, DWORD, DWORD, DWORD, LPSECURITY_ATTRIBUTES);
using NamedPipeW = HANDLE (WINAPI *)(LPCWSTR, DWORD, DWORD, DWORD, DWORD, DWORD, DWORD, LPSECURITY_ATTRIBUTES);
NamedPipeA g_pipe_a = nullptr;
NamedPipeW g_pipe_w = nullptr;
volatile LONG g_pipe_capture = 0;

bool uv_pipe_prefix(const char* name) {
    constexpr char prefix[] = "\\\\?\\pipe\\uv\\";
    if (!name) return false;
    for (size_t i = 0; i < sizeof(prefix) - 1; ++i) {
        if (!name[i] || name[i] != prefix[i]) return false;
    }
    return true;
}

bool uv_pipe_prefix(const wchar_t* name) {
    constexpr wchar_t prefix[] = L"\\\\?\\pipe\\uv\\";
    if (!name) return false;
    for (size_t i = 0; i < sizeof(prefix) / sizeof(wchar_t) - 1; ++i) {
        if (!name[i] || name[i] != prefix[i]) return false;
    }
    return true;
}

#ifdef GOGOKE_LPAC_PATH_TEST
bool (*g_test_stderr)(const char*, DWORD) = nullptr;
#endif

void report_pipe_failure(const char* api, DWORD error) {
    // Fixed text and decimal Win32 code only: never disclose a generated pipe
    // name, request, credential, or private path. Stderr is already host-held.
    char line[112] = {};
    DWORD len = 0;
    const char* lead = "gogoke Claude ";
    while (*lead) line[len++] = *lead++;
    while (*api) line[len++] = *api++;
    const char* middle = " failed win32=";
    while (*middle) line[len++] = *middle++;
    char digits[10];
    DWORD count = 0;
    do { digits[count++] = static_cast<char>('0' + error % 10); error /= 10; } while (error);
    while (count) line[len++] = digits[--count];
    const char* tail = " prefix=uv\n";
    while (*tail) line[len++] = *tail++;
#ifdef GOGOKE_LPAC_PATH_TEST
    if (g_test_stderr) { g_test_stderr(line, len); return; }
#endif
    const HANDLE stderr_handle = GetStdHandle(STD_ERROR_HANDLE);
    if (stderr_handle && stderr_handle != INVALID_HANDLE_VALUE) {
        DWORD written = 0;
        WriteFile(stderr_handle, line, len, &written, nullptr);
    }
}

HANDLE WINAPI observed_pipe_a(LPCSTR name, DWORD open_mode, DWORD pipe_mode,
    DWORD instances, DWORD out_size, DWORD in_size, DWORD timeout, LPSECURITY_ATTRIBUTES security) {
    const HANDLE result = g_pipe_a(name, open_mode, pipe_mode, instances,
        out_size, in_size, timeout, security);
    const DWORD error = GetLastError();
    if (result == INVALID_HANDLE_VALUE && uv_pipe_prefix(name) &&
        InterlockedCompareExchange(&g_pipe_capture, 1, 0) == 0)
        report_pipe_failure("CreateNamedPipeA", error);
    SetLastError(error);
    return result;
}

HANDLE WINAPI observed_pipe_w(LPCWSTR name, DWORD open_mode, DWORD pipe_mode,
    DWORD instances, DWORD out_size, DWORD in_size, DWORD timeout, LPSECURITY_ATTRIBUTES security) {
    const HANDLE result = g_pipe_w(name, open_mode, pipe_mode, instances,
        out_size, in_size, timeout, security);
    const DWORD error = GetLastError();
    if (result == INVALID_HANDLE_VALUE && uv_pipe_prefix(name) &&
        InterlockedCompareExchange(&g_pipe_capture, 1, 0) == 0)
        report_pipe_failure("CreateNamedPipeW", error);
    SetLastError(error);
    return result;
}

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

struct Mapping {
    wchar_t nt[kMaxPathUnits];
    wchar_t dos[kMaxPathUnits];
    DWORD nt_len;
    DWORD dos_len;
};

bool environment_present(const wchar_t* name) {
    SetLastError(ERROR_SUCCESS);
    const DWORD size = GetEnvironmentVariableW(name, nullptr, 0);
    return size != 0 || GetLastError() != ERROR_ENVVAR_NOT_FOUND;
}

bool read_mapping(const wchar_t* nt_name, const wchar_t* dos_name, Mapping* mapping) {
    mapping->nt_len = GetEnvironmentVariableW(nt_name, mapping->nt, kMaxPathUnits);
    mapping->dos_len = GetEnvironmentVariableW(dos_name, mapping->dos, kMaxPathUnits);
    const DWORD nt_len = mapping->nt_len, dos_len = mapping->dos_len;
    const wchar_t* nt = mapping->nt;
    const wchar_t* dos = mapping->dos;
    if (!nt_len || !dos_len || nt_len >= kMaxPathUnits ||
        dos_len >= kMaxPathUnits) return false;
    // The host supplies only handle-derived native roots. Environment data
    // selects a spelling only; it never authorizes opening a target.
    if (nt_len < 9 || nt[0] != L'\\' ||
        CompareStringOrdinal(nt, 8, L"\\Device\\", 8, TRUE) != CSTR_EQUAL)
        return false;
    if (dos_len < 7 || dos[0] != L'\\' || dos[1] != L'\\' ||
        dos[2] != L'?' || dos[3] != L'\\' ||
        !((dos[4] >= L'A' && dos[4] <= L'Z') ||
          (dos[4] >= L'a' && dos[4] <= L'z')) ||
        dos[5] != L':' || dos[6] != L'\\') return false;
    if (nt[nt_len - 1] == L'\\' || dos[dos_len - 1] == L'\\')
        return false;
    return true;
}

bool read_mappings(Mapping (&mappings)[kMaxRoots], DWORD* count) {
    wchar_t value[4] = {};
    SetLastError(ERROR_SUCCESS);
    const DWORD length = GetEnvironmentVariableW(kCountEnv, value, 4);
    if (!length) {
        if (GetLastError() != ERROR_ENVVAR_NOT_FOUND ||
            environment_present(kNtEnv1) || environment_present(kDosEnv1) ||
            environment_present(kNtEnv2) || environment_present(kDosEnv2)) return false;
        *count = 1;
    } else if (length == 1 && (value[0] == L'2' || value[0] == L'3')) {
        *count = static_cast<DWORD>(value[0]-L'0');
    } else return false;
    if (!read_mapping(kNtEnv,kDosEnv,&mappings[0])) return false;
    if (*count >= 2) {
        if (!read_mapping(kNtEnv1,kDosEnv1,&mappings[1])) return false;
    } else if (environment_present(kNtEnv1) || environment_present(kDosEnv1)) return false;
    if (*count == 3) {
        if (!read_mapping(kNtEnv2,kDosEnv2,&mappings[2])) return false;
    } else if (environment_present(kNtEnv2) || environment_present(kDosEnv2)) return false;
    for (DWORD i=0; i<*count; ++i) {
        for (DWORD j=i+1; j<*count; ++j) {
            const auto& a=mappings[i]; const auto& b=mappings[j];
            if (within_root(a.nt,a.nt_len,b.nt,b.nt_len) ||
                within_root(b.nt,b.nt_len,a.nt,a.nt_len) ||
                within_root(a.dos,a.dos_len,b.dos,b.dos_len) ||
                within_root(b.dos,b.dos_len,a.dos,a.dos_len)) return false;
        }
    }
    return true;
}

DWORD WINAPI compatible_final_path(HANDLE file, LPWSTR output,
                                   DWORD capacity, DWORD flags) {
    const DWORD result = g_original(file, output, capacity, flags);
    const DWORD original_error = GetLastError();
    if (flags != 0 || result != 0 || original_error != ERROR_ACCESS_DENIED)
        return SetLastError(original_error), result;

    // Fixed stack storage avoids heap activity and caps every Win32 length.
    Mapping mappings[kMaxRoots] = {};
    DWORD root_count = 0;
    if (!read_mappings(mappings, &root_count))
        return SetLastError(original_error), result;
    wchar_t actual_nt[kMaxPathUnits];
    const DWORD actual_len = g_original(file, actual_nt, kMaxPathUnits,
                                        VOLUME_NAME_NT);
    if (!actual_len || actual_len >= kMaxPathUnits)
        return SetLastError(original_error), result;
    const Mapping* selected=nullptr;
    for (DWORD i=0; i<root_count; ++i) {
        if (within_root(actual_nt,actual_len,mappings[i].nt,mappings[i].nt_len)) {
            if (selected) return SetLastError(original_error), result;
            selected=&mappings[i];
        }
    }
    if (!selected) return SetLastError(original_error), result;
    const DWORD suffix_len = actual_len - selected->nt_len;
    if (selected->dos_len > kMaxPathUnits - 1 - suffix_len)
        return SetLastError(original_error), result;
    const DWORD translated_len = selected->dos_len + suffix_len;
    // Win32 returns required size INCLUDING NUL for a short or zero buffer;
    // on success it returns the copied length EXCLUDING NUL.
    if (capacity <= translated_len) {
        SetLastError(ERROR_INSUFFICIENT_BUFFER);
        return translated_len + 1;
    }
    if (!output) return SetLastError(original_error), result;
    for (DWORD i = 0; i < selected->dos_len; ++i) output[i] = selected->dos[i];
    for (DWORD i = 0; i < suffix_len; ++i)
        output[selected->dos_len + i] = actual_nt[selected->nt_len + i];
    output[translated_len] = L'\0';
    SetLastError(ERROR_SUCCESS);
    return translated_len;
}

// The host independently pins image bytes before suspended launch. Resolve
// only exact named imports from the main image, never a DLL-wide code patch.
struct ImportSlots { void** first; void** second; };
bool exact_main_import_slots(BYTE* image, const char* first, const char* second,
                             ImportSlots* slots) {
    slots->first = nullptr;
    slots->second = nullptr;
    if (!image) return false;
    auto* dos = reinterpret_cast<IMAGE_DOS_HEADER*>(image);
    if (dos->e_magic != IMAGE_DOS_SIGNATURE || dos->e_lfanew <= 0 ||
        dos->e_lfanew > 4096) return false;
    auto* pe = reinterpret_cast<IMAGE_NT_HEADERS64*>(image + dos->e_lfanew);
    if (pe->Signature != IMAGE_NT_SIGNATURE ||
        pe->FileHeader.Machine != IMAGE_FILE_MACHINE_AMD64 ||
        pe->OptionalHeader.Magic != IMAGE_NT_OPTIONAL_HDR64_MAGIC ||
        pe->OptionalHeader.NumberOfRvaAndSizes <= IMAGE_DIRECTORY_ENTRY_IMPORT)
        return false;
    const DWORD image_size = pe->OptionalHeader.SizeOfImage;
    const auto imports = pe->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT];
    if (!range_ok(imports.VirtualAddress, imports.Size, image_size) ||
        imports.Size < sizeof(IMAGE_IMPORT_DESCRIPTOR)) return false;
    auto* descriptors = reinterpret_cast<IMAGE_IMPORT_DESCRIPTOR*>(
        image + imports.VirtualAddress);
    const size_t count = imports.Size / sizeof(IMAGE_IMPORT_DESCRIPTOR);
    if (count > 64) return false;
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
            !range_ok(desc.FirstThunk, sizeof(IMAGE_THUNK_DATA64), image_size)) return false;
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
                !range_ok(rva, sizeof(WORD) + 1, image_size)) return false;
            const char* name = reinterpret_cast<const char*>(image + rva + sizeof(WORD));
            if (ascii_exact(name, first, image_size - rva - sizeof(WORD))) {
                if (!kernel32 || slots->first) return false;
                slots->first = reinterpret_cast<void**>(&values[i].u1.Function);
            }
            if (second && ascii_exact(name, second, image_size - rva - sizeof(WORD))) {
                if (!kernel32 || slots->second) return false;
                slots->second = reinterpret_cast<void**>(&values[i].u1.Function);
            }
        }
        if (!ended) return false;
    }
    return descriptor_end && slots->first && *slots->first &&
        (!second || (slots->second && *slots->second));
}

void** exact_main_import_slot(BYTE* image) {
    ImportSlots slots = {};
    return exact_main_import_slots(image, "GetFinalPathNameByHandleW", nullptr, &slots)
        ? slots.first : nullptr;
}

bool patch_slot(void** slot, void* original, void* replacement) {
    DWORD old_protection = 0;
    if (!VirtualProtect(slot, sizeof(void*), PAGE_READWRITE, &old_protection)) return false;
    const void* observed = InterlockedCompareExchangePointer(
        reinterpret_cast<PVOID volatile*>(slot), replacement, original);
    DWORD ignored = 0;
    const BOOL restored = VirtualProtect(slot, sizeof(void*), old_protection, &ignored);
    return observed == original && restored;
}

bool patch_exact_main_import() {
    auto* image = static_cast<BYTE*>(static_cast<void*>(GetModuleHandleW(nullptr)));
    void** slot = exact_main_import_slot(image);
    if (!slot) return false;
    auto original = reinterpret_cast<FinalPath>(*slot);
    if (!patch_slot(slot, reinterpret_cast<void*>(original),
                    reinterpret_cast<void*>(&compatible_final_path))) return false;
    g_original = original;
    return true;
}

bool patch_exact_claude_imports() {
    auto* image = static_cast<BYTE*>(static_cast<void*>(GetModuleHandleW(nullptr)));
    ImportSlots slots = {};
    if (!exact_main_import_slots(image, "CreateNamedPipeA", "CreateNamedPipeW", &slots))
        return false;
    const auto original_a = reinterpret_cast<NamedPipeA>(*slots.first);
    const auto original_w = reinterpret_cast<NamedPipeW>(*slots.second);
    g_pipe_a = original_a;
    g_pipe_w = original_w;
    // A failed second patch prevents DLL initialization and child activation.
    if (!patch_slot(slots.first, reinterpret_cast<void*>(original_a),
                    reinterpret_cast<void*>(&observed_pipe_a)) ||
        !patch_slot(slots.second, reinterpret_cast<void*>(original_w),
                    reinterpret_cast<void*>(&observed_pipe_w))) return false;
    return true;
}

enum class CompatMode { Invalid, CodexPath, ClaudePipe };
CompatMode select_mode() {
    wchar_t mode[32] = {};
    SetLastError(ERROR_SUCCESS);
    const DWORD length = GetEnvironmentVariableW(kModeEnv, mode, 32);
    if (!length && GetLastError() == ERROR_ENVVAR_NOT_FOUND)
        return CompatMode::CodexPath;
    if (length == sizeof(kClaudeMode) / sizeof(wchar_t) - 1 &&
        lstrcmpW(mode, kClaudeMode) == 0) return CompatMode::ClaudePipe;
    return CompatMode::Invalid;
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
    switch (select_mode()) {
        case CompatMode::CodexPath: return patch_exact_main_import() ? TRUE : FALSE;
        case CompatMode::ClaudePipe: return patch_exact_claude_imports() ? TRUE : FALSE;
        default: return FALSE;
    }
}
#endif
