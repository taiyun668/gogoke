// Cloud-only behavior tests compile the exact production shim source.
#define GOGOKE_LPAC_PATH_TEST
#include "shim.cpp"
#include <stdio.h>
#include <string.h>

namespace {
constexpr wchar_t kNtRoot[] = L"\\Device\\HarddiskVolume3\\seat\\home";
constexpr wchar_t kDosRoot[] = L"\\\\?\\C:\\tmp\\seat\\home";
constexpr wchar_t kChild[] = L"\\Device\\HarddiskVolume3\\seat\\home\\auth.json";
constexpr wchar_t kDosChild[] = L"\\\\?\\C:\\tmp\\seat\\home\\auth.json";
const wchar_t* mock_nt = kChild;
DWORD mock_dos_error = ERROR_ACCESS_DENIED;
DWORD mock_nt_error = 0;
bool mock_dos_success = false;
int nt_queries = 0;
HANDLE observed_handle = nullptr;

DWORD WINAPI mock_final_path(HANDLE handle, LPWSTR out, DWORD cap, DWORD flags) {
    observed_handle = handle;
    if (flags == 0) {
        if (mock_dos_success) {
            const wchar_t value[] = L"native";
            if (out && cap >= 7) memcpy(out, value, sizeof(value));
            SetLastError(73);
            return 6;
        }
        SetLastError(mock_dos_error);
        return 0;
    }
    if (flags == VOLUME_NAME_NT) {
        ++nt_queries;
        if (mock_nt_error) {
            SetLastError(mock_nt_error);
            return 0;
        }
        const DWORD len = lstrlenW(mock_nt);
        if (cap <= len) { SetLastError(ERROR_INSUFFICIENT_BUFFER); return len + 1; }
        memcpy(out, mock_nt, (len + 1) * sizeof(wchar_t));
        SetLastError(ERROR_SUCCESS);
        return len;
    }
    SetLastError(ERROR_INVALID_PARAMETER);
    return 0;
}

int failures = 0;
#define CHECK(expr) do { if (!(expr)) { \
    fprintf(stderr, "shim_test.cpp:%d: %s failed\n", __LINE__, #expr); \
    ++failures; } } while (0)

void reset() {
    g_original = mock_final_path;
    mock_nt = kChild;
    mock_dos_error = ERROR_ACCESS_DENIED;
    mock_nt_error = 0;
    mock_dos_success = false;
    nt_queries = 0;
    observed_handle = nullptr;
    SetEnvironmentVariableW(kNtEnv, kNtRoot);
    SetEnvironmentVariableW(kDosEnv, kDosRoot);
}

void test_buffer_and_scope() {
    reset();
    wchar_t full[128] = {};
    HANDLE handle = reinterpret_cast<HANDLE>(0x1234);
    DWORD n = compatible_final_path(handle, full, 128, 0);
    CHECK(n == lstrlenW(kDosChild));
    CHECK(lstrcmpW(full, kDosChild) == 0);
    CHECK(full[n] == L'\0');
    CHECK(nt_queries == 1 && observed_handle == handle);
    CHECK(GetLastError() == ERROR_SUCCESS);

    reset();
    n = compatible_final_path(handle, nullptr, 0, 0);
    CHECK(n == lstrlenW(kDosChild) + 1);
    CHECK(GetLastError() == ERROR_INSUFFICIENT_BUFFER);

    reset();
    wchar_t short_buffer[4] = { L'X', L'X', L'X', L'X' };
    n = compatible_final_path(handle, short_buffer, 4, 0);
    CHECK(n == lstrlenW(kDosChild) + 1);
    CHECK(short_buffer[0] == L'X' && short_buffer[3] == L'X');

    reset();
    mock_nt = L"\\device\\HARDdiskvolume3\\SEAT\\HOME\\auth.json";
    n = compatible_final_path(handle, full, 128, 0);
    CHECK(n == lstrlenW(kDosChild));
    CHECK(lstrcmpW(full, L"\\\\?\\C:\\tmp\\seat\\home\\auth.json") == 0);

    reset();
    mock_nt = kNtRoot;
    n = compatible_final_path(handle, full, 128, 0);
    CHECK(n == lstrlenW(kDosRoot));
    CHECK(lstrcmpW(full, kDosRoot) == 0);

    reset();
    mock_nt = L"\\Device\\HarddiskVolume3\\seat\\homely\\auth.json";
    n = compatible_final_path(handle, full, 128, 0);
    CHECK(n == 0 && GetLastError() == ERROR_ACCESS_DENIED);

    reset();
    mock_nt = L"\\Device\\HarddiskVolume4\\seat\\home\\auth.json";
    n = compatible_final_path(handle, full, 128, 0);
    CHECK(n == 0 && GetLastError() == ERROR_ACCESS_DENIED);
}

void test_passthrough_and_invalid_mapping() {
    wchar_t output[128] = {};
    HANDLE handle = reinterpret_cast<HANDLE>(0x1234);
    reset();
    DWORD n = compatible_final_path(handle, output, 128, VOLUME_NAME_NT);
    CHECK(n == lstrlenW(kChild) && lstrcmpW(output, kChild) == 0);
    CHECK(nt_queries == 1 && GetLastError() == ERROR_SUCCESS);

    reset();
    n = compatible_final_path(handle, output, 128, VOLUME_NAME_NONE);
    CHECK(n == 0 && nt_queries == 0 && GetLastError() == ERROR_INVALID_PARAMETER);

    reset();
    mock_dos_error = ERROR_SHARING_VIOLATION;
    n = compatible_final_path(handle, output, 128, 0);
    CHECK(n == 0 && nt_queries == 0 && GetLastError() == ERROR_SHARING_VIOLATION);

    reset();
    mock_dos_success = true;
    n = compatible_final_path(handle, output, 128, 0);
    CHECK(n == 6 && lstrcmpW(output, L"native") == 0);
    CHECK(nt_queries == 0 && GetLastError() == 73);

    reset();
    SetEnvironmentVariableW(kNtEnv, nullptr);
    n = compatible_final_path(handle, output, 128, 0);
    CHECK(n == 0 && nt_queries == 0 && GetLastError() == ERROR_ACCESS_DENIED);

    reset();
    SetEnvironmentVariableW(kDosEnv, L"\\\\?\\GLOBALROOT\\Device\\HarddiskVolume3\\seat\\home");
    n = compatible_final_path(handle, output, 128, 0);
    CHECK(n == 0 && nt_queries == 0 && GetLastError() == ERROR_ACCESS_DENIED);

    reset();
    mock_nt_error = ERROR_INVALID_HANDLE;
    n = compatible_final_path(handle, output, 128, 0);
    CHECK(n == 0 && nt_queries == 1 && GetLastError() == ERROR_ACCESS_DENIED);
}

void test_iat_shape() {
    alignas(8) BYTE image[4096] = {};
    auto* dos = reinterpret_cast<IMAGE_DOS_HEADER*>(image);
    dos->e_magic = IMAGE_DOS_SIGNATURE;
    dos->e_lfanew = 0x80;
    auto* pe = reinterpret_cast<IMAGE_NT_HEADERS64*>(image + 0x80);
    pe->Signature = IMAGE_NT_SIGNATURE;
    pe->FileHeader.Machine = IMAGE_FILE_MACHINE_AMD64;
    pe->OptionalHeader.Magic = IMAGE_NT_OPTIONAL_HDR64_MAGIC;
    pe->OptionalHeader.SizeOfImage = sizeof(image);
    pe->OptionalHeader.NumberOfRvaAndSizes = 16;
    auto& imports = pe->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT];
    imports.VirtualAddress = 0x500;
    imports.Size = 2 * sizeof(IMAGE_IMPORT_DESCRIPTOR);
    auto* desc = reinterpret_cast<IMAGE_IMPORT_DESCRIPTOR*>(image + 0x500);
    desc[0].Name = 0x600;
    desc[0].FirstThunk = 0x700;
    desc[0].OriginalFirstThunk = 0x800;
    memcpy(image + 0x600, "kernel32.dll", 13);
    auto* values = reinterpret_cast<IMAGE_THUNK_DATA64*>(image + 0x700);
    auto* names = reinterpret_cast<IMAGE_THUNK_DATA64*>(image + 0x800);
    names[0].u1.AddressOfData = 0x900;
    values[0].u1.Function = 0x1234;
    memcpy(image + 0x902, "GetFinalPathNameByHandleW", 26);
    CHECK(exact_main_import_slot(image) ==
        reinterpret_cast<void**>(&values[0].u1.Function));
    desc[0].Name = 4090;
    CHECK(exact_main_import_slot(image) == nullptr);
    desc[0].Name = 0x600;
    memcpy(image + 0x600, "kernelbase.dll", 15);
    CHECK(exact_main_import_slot(image) == nullptr);
    memcpy(image + 0x600, "kernel32.dll", 13);
    imports.Size = sizeof(IMAGE_IMPORT_DESCRIPTOR);
    CHECK(exact_main_import_slot(image) == nullptr); // no terminator
    imports.Size = 2 * sizeof(IMAGE_IMPORT_DESCRIPTOR);
    pe->FileHeader.Machine = IMAGE_FILE_MACHINE_I386;
    CHECK(exact_main_import_slot(image) == nullptr);
}
} // namespace

int main() {
    test_buffer_and_scope();
    test_passthrough_and_invalid_mapping();
    test_iat_shape();
    if (failures) fprintf(stderr, "%d fixed shim tests failed\n", failures);
    return failures ? 1 : 0;
}
