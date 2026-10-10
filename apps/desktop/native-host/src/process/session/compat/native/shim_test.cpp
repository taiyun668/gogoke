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
constexpr wchar_t kNtWorktree[] = L"\\Device\\HarddiskVolume3\\seat\\worktree";
constexpr wchar_t kDosWorktree[] = L"\\\\?\\C:\\tmp\\seat\\worktree";
constexpr wchar_t kNtWorktreeChild[] = L"\\Device\\HarddiskVolume3\\seat\\worktree\\README.md";
constexpr wchar_t kDosWorktreeChild[] = L"\\\\?\\C:\\tmp\\seat\\worktree\\README.md";
constexpr wchar_t kNtSession[] = L"\\Device\\HarddiskVolume3\\seat\\session";
constexpr wchar_t kDosSession[] = L"\\\\?\\C:\\tmp\\seat\\session";
constexpr wchar_t kNtSessionChild[] = L"\\Device\\HarddiskVolume3\\seat\\session\\state.sqlite";
constexpr wchar_t kDosSessionChild[] = L"\\\\?\\C:\\tmp\\seat\\session\\state.sqlite";
const wchar_t* mock_nt = kChild;
DWORD mock_dos_error = ERROR_ACCESS_DENIED;
DWORD mock_nt_error = 0;
bool mock_dos_success = false;
int nt_queries = 0;
HANDLE observed_handle = nullptr;
HANDLE mock_pipe_result = INVALID_HANDLE_VALUE;
DWORD mock_pipe_error = ERROR_ACCESS_DENIED;
DWORD observed_open_mode = 0, observed_pipe_mode = 0, observed_instances = 0;
DWORD observed_out_size = 0, observed_in_size = 0, observed_timeout = 0;
LPSECURITY_ATTRIBUTES observed_security = nullptr;
const char* observed_name_a = nullptr;
char observed_name_a_copy[80] = {};
const wchar_t* observed_name_w = nullptr;
const char* observed_file_name = nullptr;
char observed_file_name_copy[80] = {};
DWORD observed_file_access = 0, observed_file_share = 0;
DWORD observed_file_disposition = 0, observed_file_flags = 0;
LPSECURITY_ATTRIBUTES observed_file_security = nullptr;
HANDLE observed_file_template = nullptr;
HANDLE mock_file_result = INVALID_HANDLE_VALUE;
DWORD mock_file_error = ERROR_ACCESS_DENIED;
char captured_stderr[160] = {};
DWORD captured_length = 0;
int stderr_calls = 0;
char captured_pipe_observations[12][320] = {};
int pipe_observation_calls = 0;
bool fail_pipe_observation = false;
DWORD original_file_entry_error = 0;

HANDLE WINAPI mock_named_pipe_a(LPCSTR name, DWORD open_mode, DWORD pipe_mode,
    DWORD instances, DWORD out_size, DWORD in_size, DWORD timeout, LPSECURITY_ATTRIBUTES security) {
    observed_name_a = name;
    strcpy_s(observed_name_a_copy, name);
    observed_open_mode = open_mode; observed_pipe_mode = pipe_mode;
    observed_instances = instances; observed_out_size = out_size;
    observed_in_size = in_size; observed_timeout = timeout; observed_security = security;
    SetLastError(mock_pipe_error);
    return mock_pipe_result;
}

HANDLE WINAPI mock_file_a(LPCSTR name, DWORD access, DWORD share,
    LPSECURITY_ATTRIBUTES security, DWORD disposition, DWORD flags, HANDLE template_file) {
    original_file_entry_error = GetLastError();
    observed_file_name = name;
    strcpy_s(observed_file_name_copy, name);
    observed_file_access = access; observed_file_share = share;
    observed_file_security = security; observed_file_disposition = disposition;
    observed_file_flags = flags; observed_file_template = template_file;
    SetLastError(mock_file_error);
    return mock_file_result;
}

HANDLE WINAPI mock_named_pipe_w(LPCWSTR name, DWORD open_mode, DWORD pipe_mode,
    DWORD instances, DWORD out_size, DWORD in_size, DWORD timeout, LPSECURITY_ATTRIBUTES security) {
    observed_name_w = name;
    observed_open_mode = open_mode; observed_pipe_mode = pipe_mode;
    observed_instances = instances; observed_out_size = out_size;
    observed_in_size = in_size; observed_timeout = timeout; observed_security = security;
    SetLastError(mock_pipe_error);
    return mock_pipe_result;
}

bool mock_stderr(const char* line, DWORD len) {
    ++stderr_calls;
    captured_length = len;
    memcpy(captured_stderr, line, len);
    captured_stderr[len] = 0;
    SetLastError(999); // Failed stderr must not replace the original API code.
    return true;
}

bool mock_pipe_observation(const char* line, DWORD len) {
    if (pipe_observation_calls < 12 && len < 320) {
        memcpy(captured_pipe_observations[pipe_observation_calls], line, len);
        captured_pipe_observations[pipe_observation_calls][len] = 0;
    }
    ++pipe_observation_calls;
    SetLastError(997);
    return !fail_pipe_observation;
}

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
    SetEnvironmentVariableW(kCountEnv, nullptr);
    SetEnvironmentVariableW(kNtEnv1, nullptr);
    SetEnvironmentVariableW(kDosEnv1, nullptr);
    SetEnvironmentVariableW(kNtEnv2, nullptr);
    SetEnvironmentVariableW(kDosEnv2, nullptr);
}

void dual_roots() {
    SetEnvironmentVariableW(kCountEnv,L"2");
    SetEnvironmentVariableW(kNtEnv1,kNtWorktree);
    SetEnvironmentVariableW(kDosEnv1,kDosWorktree);
}

void three_roots() {
    dual_roots();
    SetEnvironmentVariableW(kCountEnv,L"3");
    SetEnvironmentVariableW(kNtEnv2,kNtSession);
    SetEnvironmentVariableW(kDosEnv2,kDosSession);
}

void test_three_distinct_roots() {
    wchar_t output[128]={};
    const HANDLE handle=reinterpret_cast<HANDLE>(0x1234);
    reset(); three_roots(); mock_nt=kNtSessionChild;
    DWORD n=compatible_final_path(handle,output,128,0);
    CHECK(n==lstrlenW(kDosSessionChild) && lstrcmpW(output,kDosSessionChild)==0);
    CHECK(nt_queries==1 && observed_handle==handle && GetLastError()==ERROR_SUCCESS);
    reset(); three_roots(); mock_nt=kNtSessionChild;
    const DWORD exact=lstrlenW(kDosSessionChild);
    wchar_t short_buffer[128];
    for (auto& unit:short_buffer) unit=L'X';
    n=compatible_final_path(handle,short_buffer,exact,0);
    CHECK(n==exact+1 && short_buffer[0]==L'X' &&
        GetLastError()==ERROR_INSUFFICIENT_BUFFER);
    reset(); three_roots(); mock_nt=kNtSessionChild;
    n=compatible_final_path(handle,output,exact+1,0);
    CHECK(n==exact && output[exact]==L'\0' && GetLastError()==ERROR_SUCCESS);
    reset(); three_roots(); mock_nt=kNtWorktreeChild;
    n=compatible_final_path(handle,output,128,0);
    CHECK(n==lstrlenW(kDosWorktreeChild) && lstrcmpW(output,kDosWorktreeChild)==0);
    reset(); three_roots(); mock_nt=kChild;
    n=compatible_final_path(handle,output,128,0);
    CHECK(n==lstrlenW(kDosChild) && lstrcmpW(output,kDosChild)==0);
    reset(); three_roots(); mock_nt=L"\\Device\\HarddiskVolume3\\seat\\session-more\\state.sqlite";
    n=compatible_final_path(handle,output,128,0);
    CHECK(n==0 && GetLastError()==ERROR_ACCESS_DENIED);
    reset(); three_roots(); SetEnvironmentVariableW(kNtEnv2,kNtRoot);
    CHECK(compatible_final_path(handle,output,128,0)==0 && nt_queries==0 &&
        GetLastError()==ERROR_ACCESS_DENIED);
    reset(); three_roots(); SetEnvironmentVariableW(kDosEnv2,kDosWorktree);
    CHECK(compatible_final_path(handle,output,128,0)==0 && nt_queries==0 &&
        GetLastError()==ERROR_ACCESS_DENIED);
    reset(); dual_roots(); SetEnvironmentVariableW(kNtEnv2,kNtSession);
    CHECK(compatible_final_path(handle,output,128,0)==0 && nt_queries==0 &&
        GetLastError()==ERROR_ACCESS_DENIED);
    reset(); three_roots(); SetEnvironmentVariableW(kDosEnv2,nullptr);
    CHECK(compatible_final_path(handle,output,128,0)==0 && nt_queries==0 &&
        GetLastError()==ERROR_ACCESS_DENIED);
}

void test_distinct_roots_and_exact_buffer() {
    wchar_t output[128] = {};
    const HANDLE handle=reinterpret_cast<HANDLE>(0x1234);
    reset(); dual_roots();
    mock_nt=kNtWorktreeChild;
    DWORD n=compatible_final_path(handle,output,128,0);
    CHECK(n==lstrlenW(kDosWorktreeChild));
    CHECK(lstrcmpW(output,kDosWorktreeChild)==0);
    CHECK(observed_handle==handle && nt_queries==1 && GetLastError()==ERROR_SUCCESS);
    reset(); dual_roots(); mock_nt=kChild;
    n=compatible_final_path(handle,output,128,0);
    CHECK(n==lstrlenW(kDosChild) && lstrcmpW(output,kDosChild)==0);

    reset(); dual_roots(); mock_nt=kNtWorktreeChild;
    const DWORD exact=lstrlenW(kDosWorktreeChild);
    wchar_t short_buffer[128];
    for (auto& unit:short_buffer) unit=L'X';
    n=compatible_final_path(handle,short_buffer,exact,0);
    CHECK(n==exact+1 && GetLastError()==ERROR_INSUFFICIENT_BUFFER);
    CHECK(short_buffer[0]==L'X' && short_buffer[exact-1]==L'X');
    reset(); dual_roots(); mock_nt=kNtWorktreeChild;
    n=compatible_final_path(handle,output,exact+1,0);
    CHECK(n==exact && output[exact]==L'\0' && GetLastError()==ERROR_SUCCESS);
    reset(); dual_roots(); mock_nt=kNtWorktreeChild;
    n=compatible_final_path(handle,nullptr,0,0);
    CHECK(n==exact+1 && GetLastError()==ERROR_INSUFFICIENT_BUFFER);
    reset(); dual_roots(); mock_nt=kNtWorktreeChild;
    n=compatible_final_path(handle,nullptr,exact+1,0);
    CHECK(n==0 && GetLastError()==ERROR_ACCESS_DENIED);

    reset(); dual_roots(); mock_nt=L"\\Device\\HarddiskVolume3\\seat\\worktree-extra\\README.md";
    n=compatible_final_path(handle,output,128,0);
    CHECK(n==0 && GetLastError()==ERROR_ACCESS_DENIED);
    reset(); dual_roots(); mock_nt=L"\\Device\\HarddiskVolume3\\seat\\sibling\\README.md";
    n=compatible_final_path(handle,output,128,0);
    CHECK(n==0 && GetLastError()==ERROR_ACCESS_DENIED);
}

void test_malformed_and_conflicting_root_list() {
    wchar_t output[128] = {};
    const HANDLE handle=reinterpret_cast<HANDLE>(0x1234);
    reset(); SetEnvironmentVariableW(kCountEnv,L"4");
    CHECK(compatible_final_path(handle,output,128,0)==0 && nt_queries==0 &&
        GetLastError()==ERROR_ACCESS_DENIED);
    reset(); SetEnvironmentVariableW(kCountEnv,L"2");
    CHECK(compatible_final_path(handle,output,128,0)==0 && nt_queries==0 &&
        GetLastError()==ERROR_ACCESS_DENIED);
    reset(); SetEnvironmentVariableW(kNtEnv1,kNtWorktree);
    CHECK(compatible_final_path(handle,output,128,0)==0 && nt_queries==0 &&
        GetLastError()==ERROR_ACCESS_DENIED);
    reset(); dual_roots(); SetEnvironmentVariableW(kNtEnv1,kNtRoot);
    CHECK(compatible_final_path(handle,output,128,0)==0 && nt_queries==0 &&
        GetLastError()==ERROR_ACCESS_DENIED);
    reset(); dual_roots(); SetEnvironmentVariableW(kNtEnv1,L"\\Device\\HarddiskVolume3\\seat\\home\\nested");
    CHECK(compatible_final_path(handle,output,128,0)==0 && nt_queries==0 &&
        GetLastError()==ERROR_ACCESS_DENIED);
    reset(); dual_roots(); SetEnvironmentVariableW(kDosEnv1,L"\\\\?\\GLOBALROOT\\Device\\HarddiskVolume3\\seat\\worktree");
    CHECK(compatible_final_path(handle,output,128,0)==0 && nt_queries==0 &&
        GetLastError()==ERROR_ACCESS_DENIED);
    reset(); dual_roots(); SetEnvironmentVariableW(kDosEnv1,kDosRoot);
    CHECK(compatible_final_path(handle,output,128,0)==0 && nt_queries==0 &&
        GetLastError()==ERROR_ACCESS_DENIED);
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

void test_claude_pipe_observation() {
    g_pipe_a = mock_named_pipe_a;
    g_pipe_w = mock_named_pipe_w;
    g_test_stderr = mock_stderr;
    g_test_pipe_observation = mock_pipe_observation;
    g_pipe_capture = 0;
    g_server_observation = 0;
    g_server_w_observation = 0;
    g_diagnostic_write_error = 0;
    fail_pipe_observation = false;
    pipe_observation_calls = 0;
    stderr_calls = 0;
    mock_pipe_result = INVALID_HANDLE_VALUE;
    mock_pipe_error = ERROR_ACCESS_DENIED;
    SECURITY_ATTRIBUTES security = {};
    constexpr char name[] = "\\\\?\\pipe\\uv\\private-random-name";
    HANDLE result = observed_pipe_a(name, 1, 2, 3, 4, 5, 6, &security);
    CHECK(result == INVALID_HANDLE_VALUE && GetLastError() == ERROR_ACCESS_DENIED);
    CHECK(observed_name_a == name && observed_open_mode == 1 && observed_pipe_mode == 2 &&
        observed_instances == 3 && observed_out_size == 4 && observed_in_size == 5 &&
        observed_timeout == 6 && observed_security == &security);
    CHECK(stderr_calls == 1 &&
        strcmp(captured_stderr, "gogoke Claude CreateNamedPipeA failed win32=5 prefix=uv\n") == 0);
    CHECK(strstr(captured_stderr, "private") == nullptr);
    mock_pipe_error = ERROR_PIPE_BUSY;
    result = observed_pipe_a(name, 1, 2, 3, 4, 5, 6, &security);
    CHECK(result == INVALID_HANDLE_VALUE && GetLastError() == ERROR_PIPE_BUSY && stderr_calls == 1);
    g_pipe_capture = 0;
    stderr_calls = 0;
    result = observed_pipe_a("\\\\?\\pipe\\uv-other\\private", 1, 2, 3, 4, 5, 6, &security);
    CHECK(result == INVALID_HANDLE_VALUE && GetLastError() == ERROR_PIPE_BUSY && stderr_calls == 0);
    result = observed_pipe_a("\\\\?\\pipe\\other\\private", 1, 2, 3, 4, 5, 6, &security);
    CHECK(result == INVALID_HANDLE_VALUE && GetLastError() == ERROR_PIPE_BUSY && stderr_calls == 0);
    mock_pipe_result = reinterpret_cast<HANDLE>(0x1234);
    result = observed_pipe_a(name, 1, 2, 3, 4, 5, 6, &security);
    CHECK(result == mock_pipe_result && GetLastError() == ERROR_PIPE_BUSY && stderr_calls == 0);
    mock_pipe_result = INVALID_HANDLE_VALUE;
    constexpr wchar_t wide_name[] = L"\\\\?\\pipe\\uv\\private-wide-name";
    result = observed_pipe_w(wide_name, 7, 8, 9, 10, 11, 12, &security);
    CHECK(result == INVALID_HANDLE_VALUE && GetLastError() == ERROR_PIPE_BUSY);
    CHECK(observed_name_w == wide_name && observed_open_mode == 7 && observed_pipe_mode == 8 &&
        observed_instances == 9 && observed_out_size == 10 && observed_in_size == 11 &&
        observed_timeout == 12 && observed_security == &security);
    CHECK(stderr_calls == 1 &&
        strcmp(captured_stderr, "gogoke Claude CreateNamedPipeW failed win32=231 prefix=uv\n") == 0);
    g_test_stderr = nullptr;
    g_test_pipe_observation = nullptr;
}

void test_claude_local_uv_pair() {
    g_pipe_a = mock_named_pipe_a;
    g_file_a = mock_file_a;
    g_test_stderr = mock_stderr;
    g_test_pipe_observation = mock_pipe_observation;
    g_pipe_capture = 0;
    g_server_observation = 0;
    g_client_observation = 0;
    g_diagnostic_write_error = 0;
    fail_pipe_observation = false;
    pipe_observation_calls = 0;
    stderr_calls = 0;
    mock_pipe_result = reinterpret_cast<HANDLE>(0x1234);
    mock_pipe_error = 71;
    mock_file_result = reinterpret_cast<HANDLE>(0x5678);
    mock_file_error = 72;
    char source[64] = {};
    char local[80] = {};
    sprintf_s(source, "\\\\?\\pipe\\uv\\123456789-%lu", GetCurrentProcessId());
    sprintf_s(local, "\\\\.\\pipe\\LOCAL\\uv\\123456789-%lu", GetCurrentProcessId());
    constexpr DWORD server_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED |
        FILE_FLAG_FIRST_PIPE_INSTANCE | WRITE_DAC;
    constexpr DWORD client_access = GENERIC_READ | FILE_WRITE_ATTRIBUTES | WRITE_DAC;
    SECURITY_ATTRIBUTES security = {sizeof(SECURITY_ATTRIBUTES), nullptr, TRUE};
    HANDLE result = observed_pipe_a(source, server_mode, 0, 1, 65536, 65536, 0, nullptr);
    CHECK(result == mock_pipe_result && GetLastError() == 71);
    CHECK(strcmp(observed_name_a_copy, local) == 0 && observed_open_mode == server_mode &&
        observed_pipe_mode == 0 && observed_instances == 1 && observed_out_size == 65536 &&
        observed_in_size == 65536 && observed_timeout == 0 && observed_security == nullptr);
    CHECK(stderr_calls == 0);
    result = observed_file_a(source, client_access, 0, &security, OPEN_EXISTING,
        FILE_FLAG_OVERLAPPED, nullptr);
    CHECK(result == mock_file_result && GetLastError() == 72);
    CHECK(strcmp(observed_file_name_copy, local) == 0 && observed_file_access == client_access &&
        observed_file_share == 0 && observed_file_security == &security &&
        observed_file_disposition == OPEN_EXISTING &&
        observed_file_flags == FILE_FLAG_OVERLAPPED && observed_file_template == nullptr);

    mock_pipe_result = INVALID_HANDLE_VALUE;
    mock_pipe_error = ERROR_ACCESS_DENIED;
    result = observed_pipe_a(source, server_mode, 0, 1, 65536, 65536, 0, nullptr);
    CHECK(result == INVALID_HANDLE_VALUE && GetLastError() == ERROR_ACCESS_DENIED);
    CHECK(strcmp(observed_name_a_copy, local) == 0 && stderr_calls == 1);
    mock_file_result = INVALID_HANDLE_VALUE;
    mock_file_error = ERROR_FILE_NOT_FOUND;
    result = observed_file_a(source, client_access, 0, &security, OPEN_EXISTING, 0, nullptr);
    CHECK(result == INVALID_HANDLE_VALUE && GetLastError() == ERROR_FILE_NOT_FOUND);
    CHECK(strcmp(observed_file_name_copy, local) == 0);

    constexpr char foreign[] = "\\\\?\\pipe\\other\\123456789-2468";
    constexpr char malformed[] = "\\\\?\\pipe\\uv\\private-random-name";
    result = observed_pipe_a(foreign, server_mode, 0, 1, 65536, 65536, 0, nullptr);
    CHECK(observed_name_a == foreign && strcmp(observed_name_a_copy, foreign) == 0);
    result = observed_pipe_a(malformed, server_mode, 0, 1, 65536, 65536, 0, nullptr);
    CHECK(observed_name_a == malformed && strcmp(observed_name_a_copy, malformed) == 0);
    constexpr char wrong_pid[] = "\\\\?\\pipe\\uv\\123456789-0";
    result = observed_pipe_a(wrong_pid, server_mode, 0, 1, 65536, 65536, 0, nullptr);
    CHECK(observed_name_a == wrong_pid && strcmp(observed_name_a_copy, wrong_pid) == 0);
    result = observed_pipe_a(source, server_mode, 0, 2, 65536, 65536, 0, nullptr);
    CHECK(observed_name_a == source && strcmp(observed_name_a_copy, source) == 0);
    result = observed_file_a(foreign, client_access, 0, &security, OPEN_EXISTING, 0, nullptr);
    CHECK(observed_file_name == foreign && strcmp(observed_file_name_copy, foreign) == 0);
    result = observed_file_a(malformed, client_access, 0, &security, OPEN_EXISTING, 0, nullptr);
    CHECK(observed_file_name == malformed && strcmp(observed_file_name_copy, malformed) == 0);
    result = observed_file_a(source, client_access, 0, &security, CREATE_NEW, 0, nullptr);
    CHECK(observed_file_name == source && strcmp(observed_file_name_copy, source) == 0);
    g_test_stderr = nullptr;
    g_test_pipe_observation = nullptr;
}

void test_original_pair_call_diagnostics_preserve_api_semantics() {
    g_pipe_a = mock_named_pipe_a;
    g_file_a = mock_file_a;
    g_test_pipe_observation = mock_pipe_observation;
    g_server_observation = 0;
    g_server_w_observation = 0;
    g_client_observation = 0;
    g_diagnostic_write_error = 0;
    fail_pipe_observation = false;
    pipe_observation_calls = 0;
    char source[64] = {};
    sprintf_s(source, "\\\\?\\pipe\\uv\\123456789-%lu", GetCurrentProcessId());
    const DWORD server_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED |
        FILE_FLAG_FIRST_PIPE_INSTANCE | WRITE_DAC;
    mock_pipe_result = reinterpret_cast<HANDLE>(static_cast<ULONG_PTR>(0x2468));
    mock_pipe_error = 71;
    HANDLE result = observed_pipe_a(source, server_mode, 0, 1, 65536, 65536, 0, nullptr);
    CHECK(result == mock_pipe_result && GetLastError() == 71);
    CHECK(pipe_observation_calls == 2);
    CHECK(strstr(captured_pipe_observations[0], "api=CreateNamedPipeA event=enter mapped=1 outcome=pending") != nullptr);
    CHECK(strstr(captured_pipe_observations[1], "event=return mapped=1 outcome=succeeded win32=0 filetime_100ns=") != nullptr);
    CHECK(strstr(captured_pipe_observations[1], " target=pipe-qualified qualified=1") != nullptr);
    CHECK(strstr(captured_pipe_observations[0], source) == nullptr);
    SECURITY_ATTRIBUTES security = {};
    mock_file_result = INVALID_HANDLE_VALUE;
    mock_file_error = ERROR_FILE_NOT_FOUND;
    SetLastError(812);
    result = observed_file_a(source, GENERIC_READ | GENERIC_WRITE | WRITE_DAC,
        0, &security, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, nullptr);
    CHECK(result == INVALID_HANDLE_VALUE && GetLastError() == ERROR_FILE_NOT_FOUND);
    CHECK(original_file_entry_error == 812 && pipe_observation_calls == 4);
    CHECK(strstr(captured_pipe_observations[2], "api=CreateFileA event=enter mapped=1 outcome=pending") != nullptr);
    CHECK(strstr(captured_pipe_observations[3], "event=return mapped=1 outcome=failed win32=2 filetime_100ns=") != nullptr);
    CHECK(strstr(captured_pipe_observations[3], " target=pipe-qualified qualified=1") != nullptr);
    CHECK(strstr(captured_pipe_observations[3], source) == nullptr);
    result = observed_file_a(source, GENERIC_READ | GENERIC_WRITE | WRITE_DAC,
        0, &security, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, nullptr);
    CHECK(result == INVALID_HANDLE_VALUE && GetLastError() == ERROR_FILE_NOT_FOUND && pipe_observation_calls == 4);
    g_test_pipe_observation = nullptr;
}

void test_first_non_uv_a_w_calls_and_stderr_delivery_failure() {
    CHECK(strcmp(target_class("CONOUT$", false), "console") == 0 &&
        strcmp(target_class("D:\\private\\auth.json", false), "disk-root") == 0 &&
        strcmp(target_class(static_cast<const char*>(nullptr), false), "other") == 0);
    g_pipe_a = mock_named_pipe_a;
    g_pipe_w = mock_named_pipe_w;
    g_file_a = mock_file_a;
    g_test_pipe_observation = mock_pipe_observation;
    g_server_observation = 0;
    g_server_w_observation = 0;
    g_client_observation = 0;
    g_diagnostic_write_error = 0;
    fail_pipe_observation = false;
    pipe_observation_calls = 0;
    mock_pipe_result = INVALID_HANDLE_VALUE;
    mock_pipe_error = ERROR_ACCESS_DENIED;
    SECURITY_ATTRIBUTES security = {};
    constexpr char other_pipe[] = "\\\\?\\pipe\\other\\private-name";
    SetLastError(811);
    HANDLE result = observed_pipe_a(other_pipe, 1, 2, 3, 4, 5, 6, &security);
    CHECK(result == INVALID_HANDLE_VALUE && GetLastError() == ERROR_ACCESS_DENIED);
    CHECK(pipe_observation_calls == 2 &&
        strstr(captured_pipe_observations[0], "api=CreateNamedPipeA event=enter mapped=0 outcome=pending") &&
        strstr(captured_pipe_observations[1], "target=pipe-other qualified=0") &&
        strstr(captured_pipe_observations[1], "pid=") &&
        strstr(captured_pipe_observations[1], other_pipe) == nullptr);
    constexpr wchar_t wide_pipe[] = L"\\\\?\\pipe\\other\\private-wide-name";
    mock_pipe_error = ERROR_PIPE_BUSY;
    SetLastError(812);
    result = observed_pipe_w(wide_pipe, 7, 8, 9, 10, 11, 12, &security);
    CHECK(result == INVALID_HANDLE_VALUE && GetLastError() == ERROR_PIPE_BUSY);
    CHECK(pipe_observation_calls == 4 &&
        strstr(captured_pipe_observations[2], "api=CreateNamedPipeW event=enter mapped=0 outcome=pending") &&
        strstr(captured_pipe_observations[3], "target=pipe-other qualified=0") &&
        strstr(captured_pipe_observations[3], "win32=231") &&
        strstr(captured_pipe_observations[3], "private-wide-name") == nullptr);
    constexpr char disk_path[] = "C:\\private\\auth.json";
    mock_file_result = INVALID_HANDLE_VALUE;
    mock_file_error = ERROR_FILE_NOT_FOUND;
    SetLastError(813);
    result = observed_file_a(disk_path, GENERIC_READ, 0, &security,
        OPEN_EXISTING, 0, nullptr);
    CHECK(result == INVALID_HANDLE_VALUE && GetLastError() == ERROR_FILE_NOT_FOUND &&
        original_file_entry_error == 813);
    CHECK(pipe_observation_calls == 6 &&
        strstr(captured_pipe_observations[4], "api=CreateFileA event=enter mapped=0 outcome=pending") &&
        strstr(captured_pipe_observations[5], "target=disk-root qualified=0") &&
        strstr(captured_pipe_observations[5], "private") == nullptr);
    observed_pipe_a(other_pipe, 1, 2, 3, 4, 5, 6, &security);
    observed_pipe_w(wide_pipe, 7, 8, 9, 10, 11, 12, &security);
    observed_file_a(disk_path, GENERIC_READ, 0, &security, OPEN_EXISTING, 0, nullptr);
    CHECK(pipe_observation_calls == 6);

    // A failed diagnostic write cannot change the wrapped API's LastError.
    // Its first failure code is retained for a later successful stderr line.
    g_server_observation = 0;
    fail_pipe_observation = true;
    SetLastError(814);
    result = observed_pipe_a(other_pipe, 1, 2, 3, 4, 5, 6, &security);
    CHECK(result == INVALID_HANDLE_VALUE && GetLastError() == ERROR_PIPE_BUSY &&
        g_diagnostic_write_error == 997 && pipe_observation_calls == 8);
    g_client_observation = 0;
    fail_pipe_observation = false;
    result = observed_file_a(disk_path, GENERIC_READ, 0, &security,
        OPEN_EXISTING, 0, nullptr);
    CHECK(result == INVALID_HANDLE_VALUE && GetLastError() == ERROR_FILE_NOT_FOUND &&
        pipe_observation_calls == 10 &&
        strstr(captured_pipe_observations[8], "prior_stderr_write_failure_win32=997") &&
        strstr(captured_pipe_observations[9], "prior_stderr_write_failure_win32=997"));
    g_test_pipe_observation = nullptr;
}

void test_claude_iat_shape() {
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
    memcpy(image + 0x600, "KERNEL32.dll", 13);
    auto* values = reinterpret_cast<IMAGE_THUNK_DATA64*>(image + 0x700);
    auto* names = reinterpret_cast<IMAGE_THUNK_DATA64*>(image + 0x800);
    names[0].u1.AddressOfData = 0x900;
    names[1].u1.AddressOfData = 0x940;
    values[0].u1.Function = 0x1234;
    values[1].u1.Function = 0x5678;
    memcpy(image + 0x902, "CreateNamedPipeA", 17);
    memcpy(image + 0x942, "CreateNamedPipeW", 17);
    ImportSlots slots = {};
    CHECK(exact_main_import_slots(image, "CreateNamedPipeA", "CreateNamedPipeW", &slots));
    CHECK(slots.first == reinterpret_cast<void**>(&values[0].u1.Function) &&
        slots.second == reinterpret_cast<void**>(&values[1].u1.Function));
    names[1].u1.AddressOfData = 0;
    CHECK(!exact_main_import_slots(image, "CreateNamedPipeA", "CreateNamedPipeW", &slots));
    names[1].u1.AddressOfData = 0x900;
    CHECK(!exact_main_import_slots(image, "CreateNamedPipeA", "CreateNamedPipeW", &slots));
    names[1].u1.AddressOfData = 0x940;
    memcpy(image + 0x600, "kernelbase.dll", 15);
    CHECK(!exact_main_import_slots(image, "CreateNamedPipeA", "CreateNamedPipeW", &slots));
    memcpy(image + 0x600, "KERNEL32.dll", 13);
    CHECK(!exact_main_import_slots(image, "CreateFileA", nullptr, &slots));
    names[2].u1.AddressOfData = 0x980;
    values[2].u1.Function = 0x9abc;
    memcpy(image + 0x982, "CreateFileA", 12);
    CHECK(exact_main_import_slots(image, "CreateFileA", nullptr, &slots));
    CHECK(slots.first == reinterpret_cast<void**>(&values[2].u1.Function));
}

void test_mode_selection() {
    SetEnvironmentVariableW(kModeEnv, nullptr);
    CHECK(select_mode() == CompatMode::CodexPath);
    SetEnvironmentVariableW(kModeEnv, kClaudeMode);
    CHECK(select_mode() == CompatMode::ClaudePipe);
    SetEnvironmentVariableW(kModeEnv, L"CODEX_PATH");
    CHECK(select_mode() == CompatMode::Invalid);
    SetEnvironmentVariableW(kModeEnv, nullptr);
}
} // namespace

int main() {
    test_buffer_and_scope();
    test_distinct_roots_and_exact_buffer();
    test_three_distinct_roots();
    test_malformed_and_conflicting_root_list();
    test_passthrough_and_invalid_mapping();
    test_iat_shape();
    test_claude_pipe_observation();
    test_claude_local_uv_pair();
    test_original_pair_call_diagnostics_preserve_api_semantics();
    test_first_non_uv_a_w_calls_and_stderr_delivery_failure();
    test_claude_iat_shape();
    test_mode_selection();
    if (failures) fprintf(stderr, "%d fixed shim tests failed\n", failures);
    return failures ? 1 : 0;
}
