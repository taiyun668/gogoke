; Gogoke template based on Tauri CLI v2.10.1's pinned NSIS installer.nsi.
; Upstream: https://raw.githubusercontent.com/tauri-apps/tauri/tauri-cli-v2.10.1/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi
; Pinned upstream SHA-256: FE22026F68BDB3292FAB376756035496CE0A35E3D580E06EBAA6A28295916EB3
Unicode true
ManifestDPIAware true
; Add in `dpiAwareness` `PerMonitorV2` to manifest for Windows 10 1607+ (note this should not affect lower versions since they should be able to ignore this and pick up `dpiAware` `true` set by `ManifestDPIAware true`)
; Currently undocumented on NSIS's website but is in the Docs folder of source tree, see
; https://github.com/kichik/nsis/blob/5fc0b87b819a9eec006df4967d08e522ddd651c9/Docs/src/attributes.but#L286-L300
; https://github.com/tauri-apps/tauri/pull/10106
ManifestDPIAwareness PerMonitorV2

!if "{{compression}}" == "none"
  SetCompress off
!else
  ; Set the compression algorithm. We default to LZMA.
  SetCompressor /SOLID "{{compression}}"
!endif

!include MUI2.nsh
!include FileFunc.nsh
!include x64.nsh
!include WordFunc.nsh
!include "utils.nsh"
!include "FileAssociation.nsh"
!include "Win\COM.nsh"
!include "Win\Propkey.nsh"
!include "StrFunc.nsh"
${StrCase}
${StrLoc}

{{#if installer_hooks}}
!include "{{installer_hooks}}"
{{/if}}

!define WEBVIEW2APPGUID "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"

!define MANUFACTURER "{{manufacturer}}"
!define PRODUCTNAME "{{product_name}}"
!define VERSION "{{version}}"
!define VERSIONWITHBUILD "{{version_with_build}}"
!define HOMEPAGE "{{homepage}}"
!define INSTALLMODE "{{install_mode}}"
!define LICENSE "{{license}}"
!define INSTALLERICON "{{installer_icon}}"
!define SIDEBARIMAGE "{{sidebar_image}}"
!define HEADERIMAGE "{{header_image}}"
!define MAINBINARYNAME "{{main_binary_name}}"
!define MAINBINARYSRCPATH "{{main_binary_path}}"
!define BUNDLEID "{{bundle_id}}"
!define COPYRIGHT "{{copyright}}"
!define OUTFILE "{{out_file}}"
!define ARCH "{{arch}}"
!define ADDITIONALPLUGINSPATH "{{additional_plugins_path}}"
!define ALLOWDOWNGRADES "{{allow_downgrades}}"
!define DISPLAYLANGUAGESELECTOR "{{display_language_selector}}"
!define INSTALLWEBVIEW2MODE "{{install_webview2_mode}}"
!define WEBVIEW2INSTALLERARGS "{{webview2_installer_args}}"
!define WEBVIEW2BOOTSTRAPPERPATH "{{webview2_bootstrapper_path}}"
!define WEBVIEW2INSTALLERPATH "{{webview2_installer_path}}"
!define MINIMUMWEBVIEW2VERSION "{{minimum_webview2_version}}"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}"
!define MANUKEY "Software\${MANUFACTURER}"
!define MANUPRODUCTKEY "${MANUKEY}\${PRODUCTNAME}"
!define UNINSTALLERSIGNCOMMAND "{{uninstaller_sign_cmd}}"
!define ESTIMATEDSIZE "{{estimated_size}}"
!define STARTMENUFOLDER "{{start_menu_folder}}"

; Tauri patches MAINBINARYSRCPATH to its NSIS (NSS) bytes before makensis.
; Freeze that exact input for File and bind its hash into this installer.
!if "$%GOGOKE_NSIS_HASH_HELPER%" == ""
  !error "GOGOKE_NSIS_HASH_HELPER is required for the Windows installer build"
!endif
!tempfile GOGOKE_PREFLIGHT_INCLUDE
!system 'powershell.exe -NoProfile -NonInteractive -File "$%GOGOKE_NSIS_HASH_HELPER%" -Source "${MAINBINARYSRCPATH}" -Include "${GOGOKE_PREFLIGHT_INCLUDE}"' = 0
!include "${GOGOKE_PREFLIGHT_INCLUDE}"
!delfile "${GOGOKE_PREFLIGHT_INCLUDE}"
!if "$%GOGOKE_NSIS_TEST_BARRIER%" != ""
  !if "$%GITHUB_ACTIONS%" != "true"
    !error "GOGOKE_NSIS_TEST_BARRIER is CI-only"
  !endif
  !define GOGOKE_NSIS_TEST_BARRIER "$%GOGOKE_NSIS_TEST_BARRIER%"
!endif

Var PassiveMode
Var UpdateMode
Var NoShortcutMode
Var GogokeInstallDomain
Var GogokeUninstallKey
Var GogokeDefaultRoot
Var GogokeVersion
Var GogokeReceiptDomain
Var GogokeInstallInstanceId
Var GogokeLifecycleLockHandle
Var GogokeInstallParent
Var GogokePinnedDirectoryList
Var GogokeAllowDirectoryCreation
Var GogokeShortcutKind
Var GogokeShortcutFolder
Var GogokeShortcutMode
Var GogokeShortcutDuplicate
Var GogokeShortcutHandleList
Var GogokeShortcutAttributeSize
Var GogokeShortcutAttributes
Var GogokeShortcutAttributesInitialized
Var GogokeShortcutStartup
Var GogokeShortcutProcessInfo
Var GogokeShortcutCommand
Var GogokeShortcutProcess
Var GogokeShortcutThread
Var GogokeShortcutChildUnknown
Var GogokeVerifierPath
Var GogokeVerifiedHandle
Var GogokePreflightHandle
Var GogokeInstalledShellHandle
Var GogokeHashAlgorithm
Var GogokeHashHandle
Var GogokeHashBuffer
Var GogokeHashDigest
Var GogokeHashHex
Var GogokePublishSource
Var GogokePublishTarget
Var GogokePublishSourceHandle
Var GogokePublishTargetHandle
Var GogokePublishBuffer
Var GogokePinSourcePath
Var GogokePinnedSourceHandle
Var GogokeSourcePackHandle
Var GogokeSourceIndexHandle
Var GogokeSourceManifestHandle
Var GogokeSourceSignatureHandle
Var GogokeTraceStage
Var GogokeTracePath
Var GogokeTraceHandle

!if ${NSIS_PTR_SIZE} > 4
  !define GOGOKE_STARTUP_EX_SIZE 112
  !define GOGOKE_STARTUP_EX_ATTRIBUTES_OFFSET 104
  !define GOGOKE_PROCESS_INFO_SIZE 24
!else
  !define GOGOKE_STARTUP_EX_SIZE 72
  !define GOGOKE_STARTUP_EX_ATTRIBUTES_OFFSET 68
  !define GOGOKE_PROCESS_INFO_SIZE 16
!endif

Name "${PRODUCTNAME}"
BrandingText "${COPYRIGHT}"
OutFile "${OUTFILE}"

; /D= may select an already registered installation root for a full update.
; Candidate uses a different default only when /D= did not select another root.
InstallDir "$LOCALAPPDATA\gogoke"

VIProductVersion "${VERSIONWITHBUILD}"
VIAddVersionKey "ProductName" "${PRODUCTNAME}"
VIAddVersionKey "FileDescription" "${PRODUCTNAME}"
VIAddVersionKey "LegalCopyright" "${COPYRIGHT}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"

# additional plugins
!addplugindir "${ADDITIONALPLUGINSPATH}"

; Uninstaller signing command
!if "${UNINSTALLERSIGNCOMMAND}" != ""
  !uninstfinalize '${UNINSTALLERSIGNCOMMAND}'
!endif

; Handle install mode, `perUser`, `perMachine` or `both`
!if "${INSTALLMODE}" == "perMachine"
  RequestExecutionLevel admin
!endif

!if "${INSTALLMODE}" == "currentUser"
  RequestExecutionLevel user
!endif

!if "${INSTALLMODE}" == "both"
  !define MULTIUSER_MUI
  !define MULTIUSER_INSTALLMODE_INSTDIR "${PRODUCTNAME}"
  !define MULTIUSER_INSTALLMODE_COMMANDLINE
  !if "${ARCH}" == "x64"
    !define MULTIUSER_USE_PROGRAMFILES64
  !else if "${ARCH}" == "arm64"
    !define MULTIUSER_USE_PROGRAMFILES64
  !endif
  !define MULTIUSER_INSTALLMODE_DEFAULT_REGISTRY_KEY "${UNINSTKEY}"
  !define MULTIUSER_INSTALLMODE_DEFAULT_REGISTRY_VALUENAME "CurrentUser"
  !define MULTIUSER_INSTALLMODEPAGE_SHOWUSERNAME
  !define MULTIUSER_INSTALLMODE_FUNCTION RestorePreviousInstallLocation
  !define MULTIUSER_EXECUTIONLEVEL Highest
  !include MultiUser.nsh
!endif

; Installer icon
!if "${INSTALLERICON}" != ""
  !define MUI_ICON "${INSTALLERICON}"
!endif

; Installer sidebar image
!if "${SIDEBARIMAGE}" != ""
  !define MUI_WELCOMEFINISHPAGE_BITMAP "${SIDEBARIMAGE}"
!endif

; Installer header image
!if "${HEADERIMAGE}" != ""
  !define MUI_HEADERIMAGE
  !define MUI_HEADERIMAGE_BITMAP  "${HEADERIMAGE}"
!endif

; Define registry key to store installer language
!define MUI_LANGDLL_REGISTRY_ROOT "HKCU"
!define MUI_LANGDLL_REGISTRY_KEY "${MANUPRODUCTKEY}"
!define MUI_LANGDLL_REGISTRY_VALUENAME "Installer Language"

; Installer pages, must be ordered as they appear
; 1. Welcome Page
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_WELCOME

; 2. License Page (if defined)
!if "${LICENSE}" != ""
  !define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
  !insertmacro MUI_PAGE_LICENSE "${LICENSE}"
!endif

; 3. Install mode (if it is set to `both`)
!if "${INSTALLMODE}" == "both"
  !define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
  !insertmacro MULTIUSER_PAGE_INSTALLMODE
!endif

; Candidate and formal installations use separate fixed per-user roots.

; 6. Start menu shortcut page
Var AppStartMenuFolder
!if "${STARTMENUFOLDER}" != ""
  !define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
  !define MUI_STARTMENUPAGE_DEFAULTFOLDER "${STARTMENUFOLDER}"
!else
  !define MUI_PAGE_CUSTOMFUNCTION_PRE Skip
!endif
!insertmacro MUI_PAGE_STARTMENU Application $AppStartMenuFolder

; 7. Installation page
!insertmacro MUI_PAGE_INSTFILES

; 8. Finish page
;
; Don't auto jump to finish page after installation page,
; because the installation page has useful info that can be used debug any issues with the installer.
!define MUI_FINISHPAGE_NOAUTOCLOSE
; Use show readme button in the finish page as a button create a desktop shortcut
!define MUI_FINISHPAGE_SHOWREADME
!define MUI_FINISHPAGE_SHOWREADME_TEXT "$(createDesktop)"
!define MUI_FINISHPAGE_SHOWREADME_FUNCTION CreateOrUpdateDesktopShortcut
; Show run app after installation.
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_FUNCTION RunMainBinary
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_FINISH

Function RunMainBinary
  nsis_tauri_utils::RunAsUser "$INSTDIR\${MAINBINARYNAME}.exe" ""
FunctionEnd

;Languages
{{#each languages}}
!insertmacro MUI_LANGUAGE "{{this}}"
{{/each}}
!insertmacro MUI_RESERVEFILE_LANGDLL
{{#each language_files}}
  !include "{{this}}"
{{/each}}

Function .onInit
  Call SelectSignedInstallDomain

  ${GetOptions} $CMDLINE "/P" $PassiveMode
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
  ${EndIf}

  ${GetOptions} $CMDLINE "/NS" $NoShortcutMode
  ${IfNot} ${Errors}
    StrCpy $NoShortcutMode 1
  ${EndIf}

  ${GetOptions} $CMDLINE "/UPDATE" $UpdateMode
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}

  !if "${DISPLAYLANGUAGESELECTOR}" == "true"
    !insertmacro MUI_LANGDLL_DISPLAY
  !endif

  !insertmacro SetContext

  !if "${INSTALLMODE}" == "currentUser"
    ${If} $GogokeInstallDomain == "CI_CANDIDATE_RESOURCE"
    ${AndIf} $INSTDIR == "$LOCALAPPDATA\gogoke"
      StrCpy $INSTDIR "$GogokeDefaultRoot"
    ${EndIf}
  !endif


  !if "${INSTALLMODE}" == "both"
    !insertmacro MULTIUSER_INIT
  !endif
FunctionEnd

Function SelectSignedInstallDomain
  IfFileExists "$EXEDIR\gogoke-resources.windows.zip" 0 invalid_install_set
  IfFileExists "$EXEDIR\resource-index.json" 0 invalid_install_set
  StrCpy $0 0
  StrCpy $1 0
  StrCpy $2 0
  StrCpy $3 0
  IfFileExists "$EXEDIR\CANDIDATE-RESOURCES.windows" 0 +2
    StrCpy $0 1
  IfFileExists "$EXEDIR\CANDIDATE-RESOURCES.windows.sig" 0 +2
    StrCpy $1 1
  IfFileExists "$EXEDIR\SHA256SUMS.windows" 0 +2
    StrCpy $2 1
  IfFileExists "$EXEDIR\SHA256SUMS.windows.sig" 0 +2
    StrCpy $3 1

  ${If} $0 = 1
  ${AndIf} $1 = 1
  ${AndIf} $2 = 0
  ${AndIf} $3 = 0
    StrCpy $GogokeInstallDomain "CI_CANDIDATE_RESOURCE"
    StrCpy $GogokeUninstallKey "Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke-candidate"
    StrCpy $GogokeDefaultRoot "$LOCALAPPDATA\gogoke-candidate"
    StrCpy $NoShortcutMode 1
    Return
  ${EndIf}
  ${If} $0 = 0
  ${AndIf} $1 = 0
  ${AndIf} $2 = 1
  ${AndIf} $3 = 1
    StrCpy $GogokeInstallDomain "OWNER_RELEASE"
    StrCpy $GogokeUninstallKey "Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke"
    StrCpy $GogokeDefaultRoot "$LOCALAPPDATA\gogoke"
    Return
  ${EndIf}
  Goto install_domain_selected
invalid_install_set:
  Abort "The installer sibling set must contain exactly one signed candidate or formal manifest pair."
install_domain_selected:
FunctionEnd

Function AcquireGogokeLifecycleLock
  ; Pin the full physical ancestor chain before preflight or path-based writes.
  ; NSIS GetFullPathName rejects a not-yet-created target; the Win32 call
  ; performs lexical normalization without requiring the target to exist.
  System::Call 'kernel32::GetFullPathNameW(w "$INSTDIR", i ${NSIS_MAX_STRLEN}, w .r0, p 0) i .r1'
  ${If} $1 == 0
  ${OrIf} $1 >= ${NSIS_MAX_STRLEN}
    Abort "The Gogoke installation path is invalid."
  ${EndIf}
  StrCpy $INSTDIR $0
  ${GetRoot} "$INSTDIR" $1
  ${If} $1 == ""
  ${OrIf} $INSTDIR == $1
  ${OrIf} $INSTDIR == "$1\"
    Abort "The Gogoke installation root cannot be a volume root."
  ${EndIf}
  ${GetParent} "$INSTDIR" $0
  ${If} $0 == $1
    StrCpy $0 "$1\"
  ${EndIf}
  StrCpy $GogokeInstallParent $0
  StrCpy $GogokeAllowDirectoryCreation 0
  Push "$0"
  Call GogokePinDirectory
  Pop $9
  ${GetOptions} $CMDLINE "/GOGOKE_LOCK_HANDLE=" $1
  ${IfNot} ${Errors}
    ; The update coordinator passes one inherited duplicate of its already
    ; exclusive file object. A copied command line has no usable handle.
    ${If} $1 == ""
      Abort "The Gogoke update lifecycle handle is missing."
    ${EndIf}
    System::Call 'kernel32::GetHandleInformation(p $1, *i .r2) i .r3'
    ${If} $3 == 0
      Abort "The Gogoke update lifecycle handle is not inherited."
    ${EndIf}
    System::Call 'kernel32::GetFinalPathNameByHandleW(p $1, w .r2, i ${NSIS_MAX_STRLEN}, i 0) i .r3'
    ${If} $3 == 0
    ${OrIf} $3 >= ${NSIS_MAX_STRLEN}
      Abort "The Gogoke update lifecycle handle has no bounded path."
    ${EndIf}
    ; Reopen only the already-pinned parent to derive its physical path. The
    ; coordinator's exclusive inherited lock cannot itself be reopened.
    System::Call 'kernel32::CreateFileW(w "$0", i 0x80, i 3, p 0, i 3, i 0x02200000, p 0) p .r4'
    ${If} $4 == -1
      Abort "The Gogoke installation parent cannot be pinned."
    ${EndIf}
    System::Call 'kernel32::GetFinalPathNameByHandleW(p $4, w .r5, i ${NSIS_MAX_STRLEN}, i 0) i .r6'
    ${If} $6 == 0
    ${OrIf} $6 >= ${NSIS_MAX_STRLEN}
      System::Call 'kernel32::CloseHandle(p $4)'
      Abort "The Gogoke installation parent has no bounded path."
    ${EndIf}
    StrCpy $6 $5 1 -1
    ${If} $6 == "\"
      StrCpy $5 "$5gogoke-install-lifecycle.lock"
    ${Else}
      StrCpy $5 "$5\gogoke-install-lifecycle.lock"
    ${EndIf}
    System::Call 'kernel32::lstrcmpiW(w "$2", w "$5") i .r6'
    ${If} $6 != 0
      System::Call 'kernel32::CloseHandle(p $4)'
      Abort "The Gogoke update lifecycle handle points outside this installation."
    ${EndIf}
    System::Alloc 8
    Pop $5
    ${If} $5 == 0
      System::Call 'kernel32::CloseHandle(p $4)'
      Abort "The Gogoke lifecycle file cannot be inspected."
    ${EndIf}
    System::Call 'kernel32::GetFileInformationByHandleEx(p $4, i 9, p $5, i 8) i .r6'
    ${If} $6 == 0
      System::Free $5
      System::Call 'kernel32::CloseHandle(p $4)'
      Abort "The Gogoke installation parent identity is unavailable."
    ${EndIf}
    System::Call '*$5(i .r6, i .r7)'
    IntOp $6 $6 & 0x400
    ${If} $6 != 0
      System::Free $5
      System::Call 'kernel32::CloseHandle(p $4)'
      Abort "The Gogoke installation parent is a reparse point."
    ${EndIf}
    System::Call 'kernel32::GetFileInformationByHandleEx(p $1, i 9, p $5, i 8) i .r6'
    ${If} $6 == 0
      System::Free $5
      System::Call 'kernel32::CloseHandle(p $4)'
      Abort "The Gogoke lifecycle file identity is unavailable."
    ${EndIf}
    System::Call '*$5(i .r6, i .r7)'
    IntOp $6 $6 & 0x410
    System::Free $5
    ${If} $6 != 0
      System::Call 'kernel32::CloseHandle(p $4)'
      Abort "The Gogoke lifecycle handle is not a plain file."
    ${EndIf}
    ; The named leaf must also be plain. The exclusive inherited file cannot
    ; be removed after this check because it denied delete sharing.
    System::Call 'kernel32::GetFileAttributesW(w "$GogokeInstallParent\gogoke-install-lifecycle.lock") i .r6'
    IntOp $7 $6 & 0x410
    ${If} $6 == -1
    ${OrIf} $7 != 0
      System::Call 'kernel32::CloseHandle(p $4)'
      Abort "The Gogoke lifecycle path is not a plain file."
    ${EndIf}
    System::Call 'kernel32::CloseHandle(p $4)'
    StrCpy $GogokeLifecycleLockHandle $1
    Goto gogoke_lock_acquired
  ${EndIf}
  ; Ordinary acquisition binds custody to the no-follow opened leaf.
  System::Call 'kernel32::CreateFileW(w "$0\gogoke-install-lifecycle.lock", i 0xC0000000, i 0, p 0, i 4, i 0x00200080, p 0) p .r0'
  ${If} $0 == -1
    Abort "Another Gogoke install, update, or uninstall operation holds the lifecycle lock."
  ${EndIf}
  StrCpy $GogokeLifecycleLockHandle $0
  ; The parent is already pinned physically. This no-follow lock is opened
  ; with no sharing, so its named leaf cannot be replaced during this check.
  System::Call 'kernel32::GetFileAttributesW(w "$GogokeInstallParent\gogoke-install-lifecycle.lock") i .r6'
  IntOp $7 $6 & 0x410
  ${If} $6 == -1
  ${OrIf} $7 != 0
    Abort "The Gogoke lifecycle path is not a plain file."
  ${EndIf}
gogoke_lock_acquired:
  StrCpy $GogokeAllowDirectoryCreation 1
  Push "$INSTDIR"
  Call GogokePinDirectory
  Pop $9
FunctionEnd

Function GogokePinDirectory
  Exch $9
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  ; The bundle can contain many files under the same output directory. Keep
  ; one handle per path rather than one handle per template expansion.
  StrCpy $2 $GogokePinnedDirectoryList
gogoke_find_pinned_directory:
  ${If} $2 == ""
  ${OrIf} $2 == 0
    Goto gogoke_pin_new_directory
  ${EndIf}
  ; System owns the pointer-sized field layout and materializes the path as
  ; an NSIS string. No integer arithmetic on address values is required.
  System::Call '*$2(p .r3, p .r2, &w${NSIS_MAX_STRLEN} .r4)'
  System::Call 'kernel32::lstrcmpiW(w "$9", w "$4") i .r3'
  ${If} $3 == 0
    Goto gogoke_pin_done
  ${EndIf}
  Goto gogoke_find_pinned_directory
gogoke_pin_new_directory:
  ; Walk from the volume root down. A missing component is created before
  ; its no-follow handle is inspected and kept without delete sharing.
  ${GetRoot} "$9" $0
  ${If} $0 == ""
    Abort "A Gogoke installation path has no volume root."
  ${EndIf}
  StrCpy $1 "$0\"
  ${If} $9 != $0
  ${AndIf} $9 != $1
    ${GetParent} "$9" $2
    ${If} $2 == $0
      StrCpy $2 $1
    ${EndIf}
    Push "$2"
    Call GogokePinDirectory
    Pop $2
  ${EndIf}
  ${If} $GogokeAllowDirectoryCreation == 1
    System::Call 'kernel32::CreateDirectoryW(w "$9", p 0) i .r0'
  ${EndIf}
  System::Call 'kernel32::CreateFileW(w "$9", i 0x80, i 3, p 0, i 3, i 0x02200000, p 0) p .r1'
  ${If} $1 == -1
    Abort "A Gogoke installation directory cannot be pinned."
  ${EndIf}
  System::Alloc 8
  Pop $2
  ${If} $2 == 0
    System::Call 'kernel32::CloseHandle(p $1)'
    Abort "A Gogoke installation directory cannot be inspected."
  ${EndIf}
  System::Call 'kernel32::GetFileInformationByHandleEx(p $1, i 9, p $2, i 8) i .r3'
  ${If} $3 == 0
    System::Free $2
    System::Call 'kernel32::CloseHandle(p $1)'
    Abort "A Gogoke installation directory has no file attributes."
  ${EndIf}
  System::Call '*$2(i .r3, i .r4)'
  System::Free $2
  IntOp $4 $3 & 0x410
  ${If} $4 != 16
    System::Call 'kernel32::CloseHandle(p $1)'
    Abort "A Gogoke installation directory is not a plain directory."
  ${EndIf}
  System::Call '*(p $1, p $GogokePinnedDirectoryList, &w${NSIS_MAX_STRLEN} "$9") p .r2'
  ${If} $2 == 0
    System::Call 'kernel32::CloseHandle(p $1)'
    Abort "A Gogoke installation directory cannot be retained."
  ${EndIf}
  StrCpy $GogokePinnedDirectoryList $2
gogoke_pin_done:
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
  Exch $9
FunctionEnd

Function GogokeReleasePinnedDirectories
gogoke_release_next_directory:
  ${If} $GogokePinnedDirectoryList == ""
  ${OrIf} $GogokePinnedDirectoryList == 0
    Return
  ${EndIf}
  System::Call '*$GogokePinnedDirectoryList(p .r0, p .r1, &w${NSIS_MAX_STRLEN} .r3)'
  System::Call 'kernel32::CloseHandle(p $0)'
  StrCpy $2 $GogokePinnedDirectoryList
  StrCpy $GogokePinnedDirectoryList $1
  System::Free $2
  Goto gogoke_release_next_directory
FunctionEnd

Function ReleaseGogokeLifecycleLock
  ${If} $GogokeLifecycleLockHandle != ""
    System::Call 'kernel32::CloseHandle(p $GogokeLifecycleLockHandle)'
    StrCpy $GogokeLifecycleLockHandle ""
  ${EndIf}
FunctionEnd

Function .onGUIEnd
  Call GogokeReleaseVerifiedShells
  Call GogokeReleaseSourcePins
  Call ReleaseGogokeLifecycleLock
  Call GogokeReleasePinnedDirectories
FunctionEnd

Function GogokeReleaseVerifiedShells
  ${If} $GogokeInstalledShellHandle != ""
    System::Call 'kernel32::CloseHandle(p $GogokeInstalledShellHandle)'
    StrCpy $GogokeInstalledShellHandle ""
  ${EndIf}
  ${If} $GogokePreflightHandle != ""
    System::Call 'kernel32::CloseHandle(p $GogokePreflightHandle)'
    StrCpy $GogokePreflightHandle ""
  ${EndIf}
FunctionEnd

Function GogokeReleaseSourcePins
  ${If} $GogokeSourceSignatureHandle != ""
    System::Call 'kernel32::CloseHandle(p $GogokeSourceSignatureHandle)'
    StrCpy $GogokeSourceSignatureHandle ""
  ${EndIf}
  ${If} $GogokeSourceManifestHandle != ""
    System::Call 'kernel32::CloseHandle(p $GogokeSourceManifestHandle)'
    StrCpy $GogokeSourceManifestHandle ""
  ${EndIf}
  ${If} $GogokeSourceIndexHandle != ""
    System::Call 'kernel32::CloseHandle(p $GogokeSourceIndexHandle)'
    StrCpy $GogokeSourceIndexHandle ""
  ${EndIf}
  ${If} $GogokeSourcePackHandle != ""
    System::Call 'kernel32::CloseHandle(p $GogokeSourcePackHandle)'
    StrCpy $GogokeSourcePackHandle ""
  ${EndIf}
FunctionEnd

Function GogokeTraceCloudStage
  ; Bounded CI-only diagnosis. The caller selects a fresh runner-temp path;
  ; no product authority or install decision reads this observation.
  Push $0
  ReadEnvStr $0 "GITHUB_ACTIONS"
  ${If} $0 == "true"
    ReadEnvStr $GogokeTracePath "GOGOKE_NSIS_TRACE_PATH"
    ${If} $GogokeTracePath != ""
      ClearErrors
      FileOpen $GogokeTraceHandle "$GogokeTracePath" w
      ${IfNot} ${Errors}
        FileWrite $GogokeTraceHandle "$GogokeTraceStage$\r$\n"
        FileClose $GogokeTraceHandle
      ${EndIf}
    ${EndIf}
  ${EndIf}
  Pop $0
FunctionEnd

Function GogokePinReadableSource
  ; Keep the exact sibling object in place from signed preflight through copy.
  ; Callers retain the returned handle until the installer closes.
  StrCpy $GogokePinnedSourceHandle ""
  System::Call 'kernel32::CreateFileW(w "$GogokePinSourcePath", i 0x80000000, i 1, p 0, i 3, i 0x00200080, p 0) p .r0'
  ${If} $0 == -1
    Abort "A Gogoke signed-set source file is unavailable."
  ${EndIf}
  StrCpy $GogokePinnedSourceHandle $0
  System::Alloc 8
  Pop $1
  ${If} $1 == 0
    System::Call 'kernel32::CloseHandle(p $GogokePinnedSourceHandle)'
    StrCpy $GogokePinnedSourceHandle ""
    Abort "A Gogoke signed-set source could not be inspected."
  ${EndIf}
  System::Call 'kernel32::GetFileInformationByHandleEx(p $GogokePinnedSourceHandle, i 9, p $1, i 8) i .r2'
  ${If} $2 == 0
    System::Free $1
    System::Call 'kernel32::CloseHandle(p $GogokePinnedSourceHandle)'
    StrCpy $GogokePinnedSourceHandle ""
    Abort "A Gogoke signed-set source could not be inspected."
  ${EndIf}
  System::Call '*$1(i .r2, i .r3)'
  IntOp $2 $2 & 0x410
  System::Free $1
  ${If} $2 != 0
    System::Call 'kernel32::CloseHandle(p $GogokePinnedSourceHandle)'
    StrCpy $GogokePinnedSourceHandle ""
    Abort "A Gogoke signed-set source is not a plain file."
  ${EndIf}
FunctionEnd

Function GogokeReleaseHashState
  ${If} $GogokeHashHandle != ""
    System::Call 'bcrypt::BCryptDestroyHash(p $GogokeHashHandle) i'
    StrCpy $GogokeHashHandle ""
  ${EndIf}
  ${If} $GogokeHashAlgorithm != ""
    System::Call 'bcrypt::BCryptCloseAlgorithmProvider(p $GogokeHashAlgorithm, i 0) i'
    StrCpy $GogokeHashAlgorithm ""
  ${EndIf}
  ${If} $GogokeHashBuffer != ""
    System::Free $GogokeHashBuffer
    StrCpy $GogokeHashBuffer ""
  ${EndIf}
  ${If} $GogokeHashDigest != ""
    System::Free $GogokeHashDigest
    StrCpy $GogokeHashDigest ""
  ${EndIf}
  ${If} $GogokeHashHex != ""
    System::Free $GogokeHashHex
    StrCpy $GogokeHashHex ""
  ${EndIf}
FunctionEnd

Function GogokeVerifyShellFile
  ; The handle denies writes and deletion until preflight or installed-shell
  ; execution has returned. A same-name replacement after File cannot pass.
  StrCpy $GogokeVerifiedHandle ""
  StrCpy $GogokeHashAlgorithm ""
  StrCpy $GogokeHashHandle ""
  StrCpy $GogokeHashBuffer ""
  StrCpy $GogokeHashDigest ""
  StrCpy $GogokeHashHex ""
  System::Call 'kernel32::CreateFileW(w "$GogokeVerifierPath", i 0x80000000, i 1, p 0, i 3, i 0x00200080, p 0) p .r0'
  ${If} $0 == -1
    Goto gogoke_verify_shell_failed
  ${EndIf}
  StrCpy $GogokeVerifiedHandle $0
  System::Alloc 65536
  Pop $GogokeHashBuffer
  ${If} $GogokeHashBuffer == 0
    Goto gogoke_verify_shell_failed
  ${EndIf}
  System::Call 'kernel32::GetFileInformationByHandleEx(p $GogokeVerifiedHandle, i 9, p $GogokeHashBuffer, i 8) i .r0'
  ${If} $0 == 0
    Goto gogoke_verify_shell_failed
  ${EndIf}
  System::Call '*$GogokeHashBuffer(i .r0, i .r1)'
  IntOp $0 $0 & 0x410
  ${If} $0 != 0
    Goto gogoke_verify_shell_failed
  ${EndIf}
  System::Call 'kernel32::GetFileSizeEx(p $GogokeVerifiedHandle, p $GogokeHashBuffer) i .r0'
  ${If} $0 == 0
    Goto gogoke_verify_shell_failed
  ${EndIf}
  System::Call '*$GogokeHashBuffer(i .r0, i .r1)'
  ${If} $1 != 0
  ${OrIf} $0 != ${GOGOKE_PREFLIGHT_LENGTH}
    Goto gogoke_verify_shell_failed
  ${EndIf}
  System::Call 'bcrypt::BCryptOpenAlgorithmProvider(*p .r0, w "SHA256", p 0, i 0) i .r1'
  ${If} $1 != 0
    Goto gogoke_verify_shell_failed
  ${EndIf}
  StrCpy $GogokeHashAlgorithm $0
  System::Call 'bcrypt::BCryptCreateHash(p $GogokeHashAlgorithm, *p .r0, p 0, i 0, p 0, i 0, i 0) i .r1'
  ${If} $1 != 0
    Goto gogoke_verify_shell_failed
  ${EndIf}
  StrCpy $GogokeHashHandle $0
gogoke_verify_shell_read:
  System::Call 'kernel32::ReadFile(p $GogokeVerifiedHandle, p $GogokeHashBuffer, i 65536, *i .r0, p 0) i .r1'
  ${If} $1 == 0
    Goto gogoke_verify_shell_failed
  ${EndIf}
  ${If} $0 == 0
    Goto gogoke_verify_shell_finish
  ${EndIf}
  System::Call 'bcrypt::BCryptHashData(p $GogokeHashHandle, p $GogokeHashBuffer, i $0, i 0) i .r1'
  ${If} $1 != 0
    Goto gogoke_verify_shell_failed
  ${EndIf}
  Goto gogoke_verify_shell_read
gogoke_verify_shell_finish:
  System::Alloc 32
  Pop $GogokeHashDigest
  ${If} $GogokeHashDigest == 0
    Goto gogoke_verify_shell_failed
  ${EndIf}
  System::Call 'bcrypt::BCryptFinishHash(p $GogokeHashHandle, p $GogokeHashDigest, i 32, i 0) i .r0'
  ${If} $0 != 0
    Goto gogoke_verify_shell_failed
  ${EndIf}
  System::Alloc 130
  Pop $GogokeHashHex
  ${If} $GogokeHashHex == 0
    Goto gogoke_verify_shell_failed
  ${EndIf}
  System::Call 'crypt32::CryptBinaryToStringW(p $GogokeHashDigest, i 32, i 0x4000000C, p $GogokeHashHex, *i 65) i .r0'
  ${If} $0 == 0
    Goto gogoke_verify_shell_failed
  ${EndIf}
  System::Call '*$GogokeHashHex(&w65 .r0)'
  System::Call 'kernel32::lstrcmpiW(w "$0", w "${GOGOKE_PREFLIGHT_SHA256}") i .r1'
  ${If} $1 != 0
    Goto gogoke_verify_shell_failed
  ${EndIf}
  Call GogokeReleaseHashState
  Return
gogoke_verify_shell_failed:
  Call GogokeReleaseHashState
  ${If} $GogokeVerifiedHandle != ""
    System::Call 'kernel32::CloseHandle(p $GogokeVerifiedHandle)'
    StrCpy $GogokeVerifiedHandle ""
  ${EndIf}
  Abort "The bundled Gogoke verifier does not match the installer payload bytes."
FunctionEnd

Function GogokePublishFile
  ; All final product leaves are opened with CREATE_NEW and no reparse follow.
  ; An ordinary leaf or dangling symlink at the final name makes this fail.
  StrCpy $GogokePublishSourceHandle ""
  StrCpy $GogokePublishTargetHandle ""
  StrCpy $GogokePublishBuffer ""
  StrCpy $GogokeTraceStage "publish-source-open"
  Call GogokeTraceCloudStage
  System::Call 'kernel32::CreateFileW(w "$GogokePublishSource", i 0x80000000, i 1, p 0, i 3, i 0x00200080, p 0) p .r0'
  ${If} $0 == -1
    System::Call 'kernel32::GetLastError() i .r1'
    StrCpy $GogokeTraceStage "publish-source-open-failed:$1"
    Call GogokeTraceCloudStage
    Goto gogoke_publish_failed
  ${EndIf}
  StrCpy $GogokePublishSourceHandle $0
  System::Alloc 65536
  Pop $GogokePublishBuffer
  ${If} $GogokePublishBuffer == 0
    Goto gogoke_publish_failed
  ${EndIf}
  System::Call 'kernel32::GetFileInformationByHandleEx(p $GogokePublishSourceHandle, i 9, p $GogokePublishBuffer, i 8) i .r0'
  ${If} $0 == 0
    Goto gogoke_publish_failed
  ${EndIf}
  System::Call '*$GogokePublishBuffer(i .r0, i .r1)'
  IntOp $0 $0 & 0x410
  ${If} $0 != 0
    Goto gogoke_publish_failed
  ${EndIf}
  StrCpy $GogokeTraceStage "publish-target-create"
  Call GogokeTraceCloudStage
  System::Call 'kernel32::CreateFileW(w "$GogokePublishTarget", i 0x40000000, i 0, p 0, i 1, i 0x00200080, p 0) p .r0'
  ${If} $0 == -1
    System::Call 'kernel32::GetLastError() i .r1'
    StrCpy $GogokeTraceStage "publish-target-create-failed:$1"
    Call GogokeTraceCloudStage
    Goto gogoke_publish_failed
  ${EndIf}
  StrCpy $GogokePublishTargetHandle $0
  StrCpy $GogokeTraceStage "publish-copy"
  Call GogokeTraceCloudStage
gogoke_publish_read:
  System::Call 'kernel32::ReadFile(p $GogokePublishSourceHandle, p $GogokePublishBuffer, i 65536, *i .r0, p 0) i .r1'
  ${If} $1 == 0
    Goto gogoke_publish_failed
  ${EndIf}
  ${If} $0 == 0
    Goto gogoke_publish_done
  ${EndIf}
  System::Call 'kernel32::WriteFile(p $GogokePublishTargetHandle, p $GogokePublishBuffer, i $0, *i .r1, p 0) i .r2'
  ${If} $2 == 0
  ${OrIf} $1 != $0
    Goto gogoke_publish_failed
  ${EndIf}
  Goto gogoke_publish_read
gogoke_publish_done:
  StrCpy $GogokeTraceStage "publish-flush"
  Call GogokeTraceCloudStage
  System::Call 'kernel32::FlushFileBuffers(p $GogokePublishTargetHandle) i .r0'
  ${If} $0 == 0
    Goto gogoke_publish_failed
  ${EndIf}
  Call GogokeReleasePublishState
  Return
gogoke_publish_failed:
  Call GogokeReleasePublishState
  Abort "A Gogoke installation leaf already exists or could not be published."
FunctionEnd

Function GogokeReleasePublishState
  ${If} $GogokePublishTargetHandle != ""
    System::Call 'kernel32::CloseHandle(p $GogokePublishTargetHandle)'
    StrCpy $GogokePublishTargetHandle ""
  ${EndIf}
  ${If} $GogokePublishSourceHandle != ""
    System::Call 'kernel32::CloseHandle(p $GogokePublishSourceHandle)'
    StrCpy $GogokePublishSourceHandle ""
  ${EndIf}
  ${If} $GogokePublishBuffer != ""
    System::Free $GogokePublishBuffer
    StrCpy $GogokePublishBuffer ""
  ${EndIf}
FunctionEnd


Section EarlyChecks
  Call AcquireGogokeLifecycleLock
  StrCpy $GogokeTraceStage "lifecycle-locked"
  Call GogokeTraceCloudStage
  ; Abort silent installer if downgrades is disabled
  !if "${ALLOWDOWNGRADES}" == "false"
  ${If} ${Silent}
    ; If downgrading
    ${If} $R0 = -1
      System::Call 'kernel32::AttachConsole(i -1)i.r0'
      ${If} $0 <> 0
        System::Call 'kernel32::GetStdHandle(i -11)i.r0'
        System::call 'kernel32::SetConsoleTextAttribute(i r0, i 0x0004)' ; set red color
        FileWrite $0 "$(silentDowngrades)"
      ${EndIf}
      Abort
    ${EndIf}
  ${EndIf}
  !endif

SectionEnd

Section SignedInstallSetPreflight
  ; Verify the sibling set with the bundled shell before touching an existing install.
  Push "$EXEDIR"
  Call GogokePinDirectory
  Pop $9
  StrCpy $GogokeTraceStage "source-directory-pinned"
  Call GogokeTraceCloudStage
  StrCpy $GogokePinSourcePath "$EXEDIR\gogoke-resources.windows.zip"
  Call GogokePinReadableSource
  StrCpy $GogokeSourcePackHandle $GogokePinnedSourceHandle
  StrCpy $GogokeTraceStage "source-pack-pinned"
  Call GogokeTraceCloudStage
  StrCpy $GogokePinSourcePath "$EXEDIR\resource-index.json"
  Call GogokePinReadableSource
  StrCpy $GogokeSourceIndexHandle $GogokePinnedSourceHandle
  StrCpy $GogokeTraceStage "source-index-pinned"
  Call GogokeTraceCloudStage
  ${If} $GogokeInstallDomain == "CI_CANDIDATE_RESOURCE"
    StrCpy $GogokePinSourcePath "$EXEDIR\CANDIDATE-RESOURCES.windows"
    Call GogokePinReadableSource
    StrCpy $GogokeSourceManifestHandle $GogokePinnedSourceHandle
    StrCpy $GogokePinSourcePath "$EXEDIR\CANDIDATE-RESOURCES.windows.sig"
  ${Else}
    StrCpy $GogokePinSourcePath "$EXEDIR\SHA256SUMS.windows"
    Call GogokePinReadableSource
    StrCpy $GogokeSourceManifestHandle $GogokePinnedSourceHandle
    StrCpy $GogokePinSourcePath "$EXEDIR\SHA256SUMS.windows.sig"
  ${EndIf}
  StrCpy $GogokeTraceStage "source-manifest-pinned"
  Call GogokeTraceCloudStage
  Call GogokePinReadableSource
  StrCpy $GogokeSourceSignatureHandle $GogokePinnedSourceHandle
  StrCpy $GogokeTraceStage "source-signature-pinned"
  Call GogokeTraceCloudStage
  InitPluginsDir
  SetOutPath "$PLUGINSDIR\gogoke-preflight"
  Push "$PLUGINSDIR\gogoke-preflight"
  Call GogokePinDirectory
  Pop $9
  StrCpy $GogokeTraceStage "preflight-scratch-pinned"
  Call GogokeTraceCloudStage
  IfFileExists "$PLUGINSDIR\gogoke-preflight\gogoke.exe" 0 +2
    Abort "The Gogoke preflight scratch leaf already exists."
  SetOverwrite off
  ClearErrors
  File /oname=gogoke.exe "${GOGOKE_PREFLIGHT_SNAPSHOT}"
  IfErrors gogoke_preflight_extract_failed
  StrCpy $GogokeTraceStage "preflight-shell-extracted"
  Call GogokeTraceCloudStage
  StrCpy $GogokeVerifierPath "$PLUGINSDIR\gogoke-preflight\gogoke.exe"
  Call GogokeVerifyShellFile
  StrCpy $GogokePreflightHandle $GogokeVerifiedHandle
  StrCpy $GogokeTraceStage "preflight-shell-hash-verified"
  Call GogokeTraceCloudStage
  ExecWait '"$PLUGINSDIR\gogoke-preflight\gogoke.exe" "--gogoke-verify-install-set=$EXEDIR" "--gogoke-install-target=$INSTDIR"' $0
  ${If} $0 != 0
    Abort "The signed Gogoke installer sibling set could not be verified."
  ${EndIf}
  StrCpy $GogokeTraceStage "signed-install-set-preflight-complete"
  Call GogokeTraceCloudStage
  SetOverwrite on
  Goto gogoke_preflight_done
gogoke_preflight_extract_failed:
  Abort "The bundled Gogoke verifier could not be extracted."
gogoke_preflight_done:
SectionEnd
!delfile "${GOGOKE_PREFLIGHT_SNAPSHOT}"

Section WebView2
  StrCpy $GogokeTraceStage "webview2-check-entered"
  Call GogokeTraceCloudStage
  ; Check if Webview2 is already installed and skip this section
  ${If} ${RunningX64}
    ReadRegStr $4 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${Else}
    ReadRegStr $4 HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${EndIf}
  ${If} $4 == ""
    ReadRegStr $4 HKCU "SOFTWARE\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${EndIf}
  ${If} $4 == ""
    StrCpy $GogokeTraceStage "webview2-runtime-absent"
  ${Else}
    StrCpy $GogokeTraceStage "webview2-runtime-present"
  ${EndIf}
  Call GogokeTraceCloudStage

  ${If} $4 == ""
    ; Webview2 installation
    ;
    ; Skip if updating
    ${If} $UpdateMode <> 1
      !if "${INSTALLWEBVIEW2MODE}" == "downloadBootstrapper"
        StrCpy $GogokeTraceStage "webview2-bootstrapper-download-start"
        Call GogokeTraceCloudStage
        Delete "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        DetailPrint "$(webview2Downloading)"
        NSISdl::download "https://go.microsoft.com/fwlink/p/?LinkId=2124703" "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        Pop $0
        ${If} $0 == "success"
          StrCpy $GogokeTraceStage "webview2-bootstrapper-download-complete"
          Call GogokeTraceCloudStage
          DetailPrint "$(webview2DownloadSuccess)"
        ${Else}
          StrCpy $GogokeTraceStage "webview2-bootstrapper-download-failed"
          Call GogokeTraceCloudStage
          DetailPrint "$(webview2DownloadError)"
          Abort "$(webview2AbortError)"
        ${EndIf}
        StrCpy $6 "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        Goto install_webview2
      !endif

      !if "${INSTALLWEBVIEW2MODE}" == "embedBootstrapper"
        Delete "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        File "/oname=$TEMP\MicrosoftEdgeWebview2Setup.exe" "${WEBVIEW2BOOTSTRAPPERPATH}"
        DetailPrint "$(installingWebview2)"
        StrCpy $6 "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        Goto install_webview2
      !endif

      !if "${INSTALLWEBVIEW2MODE}" == "offlineInstaller"
        Delete "$TEMP\MicrosoftEdgeWebView2RuntimeInstaller.exe"
        File "/oname=$TEMP\MicrosoftEdgeWebView2RuntimeInstaller.exe" "${WEBVIEW2INSTALLERPATH}"
        DetailPrint "$(installingWebview2)"
        StrCpy $6 "$TEMP\MicrosoftEdgeWebView2RuntimeInstaller.exe"
        Goto install_webview2
      !endif

      Goto webview2_done

      install_webview2:
        StrCpy $GogokeTraceStage "webview2-install-start"
        Call GogokeTraceCloudStage
        DetailPrint "$(installingWebview2)"
        ; $6 holds the path to the webview2 installer
        ExecWait "$6 ${WEBVIEW2INSTALLERARGS} /install" $1
        ${If} $1 = 0
          StrCpy $GogokeTraceStage "webview2-install-complete"
          Call GogokeTraceCloudStage
          DetailPrint "$(webview2InstallSuccess)"
        ${Else}
          StrCpy $GogokeTraceStage "webview2-install-failed"
          Call GogokeTraceCloudStage
          DetailPrint "$(webview2InstallError)"
          Abort "$(webview2AbortError)"
        ${EndIf}
      webview2_done:
    ${EndIf}
  ${Else}
    !if "${MINIMUMWEBVIEW2VERSION}" != ""
      ${VersionCompare} "${MINIMUMWEBVIEW2VERSION}" "$4" $R0
      ${If} $R0 = 1
        update_webview:
          DetailPrint "$(installingWebview2)"
          ${If} ${RunningX64}
            ReadRegStr $R1 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate" "path"
          ${Else}
            ReadRegStr $R1 HKLM "SOFTWARE\Microsoft\EdgeUpdate" "path"
          ${EndIf}
          ${If} $R1 == ""
            ReadRegStr $R1 HKCU "SOFTWARE\Microsoft\EdgeUpdate" "path"
          ${EndIf}
          ${If} $R1 != ""
            ; Chromium updater docs: https://source.chromium.org/chromium/chromium/src/+/main:docs/updater/user_manual.md
            ; Modified from "HKEY_LOCAL_MACHINE\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Microsoft EdgeWebView\ModifyPath"
            ExecWait `"$R1" /install appguid=${WEBVIEW2APPGUID}&needsadmin=true` $1
            ${If} $1 = 0
              DetailPrint "$(webview2InstallSuccess)"
            ${Else}
              MessageBox MB_ICONEXCLAMATION|MB_ABORTRETRYIGNORE "$(webview2InstallError)" IDIGNORE ignore IDRETRY update_webview
              Quit
              ignore:
            ${EndIf}
          ${EndIf}
      ${EndIf}
    !endif
  ${EndIf}
  StrCpy $GogokeTraceStage "webview2-section-complete"
  Call GogokeTraceCloudStage
SectionEnd

Section Install
  StrCpy $GogokeTraceStage "install-entered"
  Call GogokeTraceCloudStage
  ; Tauri's resources_dirs contains the resource output parents. Pin each
  ; distinct directory before the first bundled payload write.
  {{#each resources_dirs}}
    Push "$INSTDIR\{{this}}"
    Call GogokePinDirectory
    Pop $9
  {{/each}}
  StrCpy $GogokeTraceStage "install-resource-directories-pinned"
  Call GogokeTraceCloudStage
  {{#each binaries}}
    ${GetParent} "$INSTDIR\{{this}}" $0
    Push "$0"
    Call GogokePinDirectory
    Pop $9
  {{/each}}
  StrCpy $GogokeTraceStage "install-binary-directories-pinned"
  Call GogokeTraceCloudStage
  ; Scratch parents are also pinned. SetOverwrite off avoids intentional
  ; scratch replacement; it does not prove that NSIS extracted a raced leaf.
  ; Publication checks the source object and creates each final leaf atomically.
  {{#each resources}}
    ${GetParent} "$PLUGINSDIR\gogoke-payload\{{this.[1]}}" $0
    Push "$0"
    Call GogokePinDirectory
    Pop $9
  {{/each}}
  StrCpy $GogokeTraceStage "install-resource-scratch-directories-pinned"
  Call GogokeTraceCloudStage
  {{#each binaries}}
    ${GetParent} "$PLUGINSDIR\gogoke-payload\{{this}}" $0
    Push "$0"
    Call GogokePinDirectory
    Pop $9
  {{/each}}
  StrCpy $GogokeTraceStage "install-all-directories-pinned"
  Call GogokeTraceCloudStage
  SetOutPath $INSTDIR

  !ifmacrodef NSIS_HOOK_PREINSTALL
    !insertmacro NSIS_HOOK_PREINSTALL
  !endif

  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  StrCpy $GogokeTraceStage "install-app-running-check-complete"
  Call GogokeTraceCloudStage

  ; Publish the exact NSS shell already verified and pinned for preflight.
  !ifdef GOGOKE_NSIS_TEST_BARRIER
    ; A dedicated cloud mutation build can insert a closed-handle leaf after
    ; all preflight checks, immediately before the first CREATE_NEW publish.
    ClearErrors
    FileOpen $0 "${GOGOKE_NSIS_TEST_BARRIER}.ready" w
    IfErrors gogoke_test_barrier_failed
    FileClose $0
    StrCpy $0 0
  gogoke_test_barrier_wait:
    IfFileExists "${GOGOKE_NSIS_TEST_BARRIER}.go" gogoke_test_barrier_done
    Sleep 100
    IntOp $0 $0 + 1
    ${If} $0 >= 1200
      Goto gogoke_test_barrier_failed
    ${EndIf}
    Goto gogoke_test_barrier_wait
  gogoke_test_barrier_failed:
    Abort "The CI installation leaf barrier was not released."
  gogoke_test_barrier_done:
  !endif
  StrCpy $GogokePublishSource "$PLUGINSDIR\gogoke-preflight\gogoke.exe"
  StrCpy $GogokePublishTarget "$INSTDIR\${MAINBINARYNAME}.exe"
  StrCpy $GogokeTraceStage "install-first-leaf-publish-start"
  Call GogokeTraceCloudStage
  Call GogokePublishFile
  StrCpy $GogokeTraceStage "install-first-leaf-publish-complete"
  Call GogokeTraceCloudStage
  StrCpy $GogokeVerifierPath "$INSTDIR\${MAINBINARYNAME}.exe"
  Call GogokeVerifyShellFile
  StrCpy $GogokeInstalledShellHandle $GogokeVerifiedHandle

  ; Copy resources
  SetOverwrite off
  {{#each resources_dirs}}
    CreateDirectory "$INSTDIR\\{{this}}"
  {{/each}}
  {{#each resources}}
    IfFileExists "$PLUGINSDIR\gogoke-payload\{{this.[1]}}" 0 +2
      Abort "A Gogoke payload scratch leaf already exists."
    SetOutPath "$PLUGINSDIR\gogoke-payload"
    ClearErrors
    File /a "/oname={{this.[1]}}" "{{no-escape @key}}"
    IfErrors gogoke_payload_extract_failed
    StrCpy $GogokePublishSource "$PLUGINSDIR\gogoke-payload\{{this.[1]}}"
    StrCpy $GogokePublishTarget "$INSTDIR\{{this.[1]}}"
    Call GogokePublishFile
  {{/each}}

  ; Copy external binaries
  {{#each binaries}}
    IfFileExists "$PLUGINSDIR\gogoke-payload\{{this}}" 0 +2
      Abort "A Gogoke payload scratch leaf already exists."
    SetOutPath "$PLUGINSDIR\gogoke-payload"
    ClearErrors
    File /a "/oname={{this}}" "{{no-escape @key}}"
    IfErrors gogoke_payload_extract_failed
    StrCpy $GogokePublishSource "$PLUGINSDIR\gogoke-payload\{{this}}"
    StrCpy $GogokePublishTarget "$INSTDIR\{{this}}"
    Call GogokePublishFile
  {{/each}}
  SetOverwrite on

  ; Copy the already-signed sibling set without embedding or rewriting its sidecars.
  StrCpy $GogokePublishSource "$EXEDIR\gogoke-resources.windows.zip"
  StrCpy $GogokePublishTarget "$INSTDIR\gogoke-resources.windows.zip"
  Call GogokePublishFile
  StrCpy $GogokePublishSource "$EXEDIR\resource-index.json"
  StrCpy $GogokePublishTarget "$INSTDIR\resource-index.json"
  Call GogokePublishFile
  ${If} $GogokeInstallDomain == "CI_CANDIDATE_RESOURCE"
    StrCpy $GogokePublishSource "$EXEDIR\CANDIDATE-RESOURCES.windows"
    StrCpy $GogokePublishTarget "$INSTDIR\CANDIDATE-RESOURCES.windows"
    Call GogokePublishFile
    StrCpy $GogokePublishSource "$EXEDIR\CANDIDATE-RESOURCES.windows.sig"
    StrCpy $GogokePublishTarget "$INSTDIR\CANDIDATE-RESOURCES.windows.sig"
    Call GogokePublishFile
  ${Else}
    StrCpy $GogokePublishSource "$EXEDIR\SHA256SUMS.windows"
    StrCpy $GogokePublishTarget "$INSTDIR\SHA256SUMS.windows"
    Call GogokePublishFile
    StrCpy $GogokePublishSource "$EXEDIR\SHA256SUMS.windows.sig"
    StrCpy $GogokePublishTarget "$INSTDIR\SHA256SUMS.windows.sig"
    Call GogokePublishFile
  ${EndIf}

  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" "--gogoke-install-resources=$EXEDIR"' $0
  ${If} $0 != 0
    Abort "The signed Gogoke install set could not be verified and installed."
  ${EndIf}
  ReadINIStr $GogokeVersion "$INSTDIR\gogoke-install-receipt.ini" "Gogoke" "Version"
  ReadINIStr $GogokeReceiptDomain "$INSTDIR\gogoke-install-receipt.ini" "Gogoke" "Domain"
  ${If} $GogokeVersion == ""
    Abort "The Gogoke installer did not produce a verified version receipt."
  ${EndIf}
  ${If} $GogokeReceiptDomain != $GogokeInstallDomain
    Abort "The verified Gogoke set domain does not match the selected install root."
  ${EndIf}
  ClearErrors
  GetTempFileName $GogokeInstallInstanceId "$TEMP"
  IfErrors install_instance_id_failed
  Delete "$GogokeInstallInstanceId"
  Goto install_set_verified

gogoke_payload_extract_failed:
  Abort "Could not extract a bundled Gogoke payload into scratch space."
install_instance_id_failed:
  Abort "Could not create a unique Gogoke installation instance id."
install_set_verified:

  ; Create file associations
  {{#each file_associations as |association| ~}}
    {{#each association.ext as |ext| ~}}
       !insertmacro APP_ASSOCIATE "{{ext}}" "{{or association.name ext}}" "{{association-description association.description ext}}" "$INSTDIR\${MAINBINARYNAME}.exe,0" "Open with ${PRODUCTNAME}" "$INSTDIR\${MAINBINARYNAME}.exe $\"%1$\""
    {{/each}}
  {{/each}}

  ; Register deep links
  {{#each deep_link_protocols as |protocol| ~}}
    WriteRegStr SHCTX "Software\Classes\\{{protocol}}" "URL Protocol" ""
    WriteRegStr SHCTX "Software\Classes\\{{protocol}}" "" "URL:${BUNDLEID} protocol"
    WriteRegStr SHCTX "Software\Classes\\{{protocol}}\DefaultIcon" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
    WriteRegStr SHCTX "Software\Classes\\{{protocol}}\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
  {{/each}}

  !if "${INSTALLMODE}" == "both"
    ; Save install mode to be selected by default for the next installation such as updating
    ; or when uninstalling
    WriteRegStr SHCTX "$GogokeUninstallKey" $MultiUser.InstallMode 1
  !endif

  ; Save current MAINBINARYNAME for future updates
  ClearErrors
  WriteRegStr HKCU "$GogokeUninstallKey" "MainBinaryName" "${MAINBINARYNAME}.exe"

  ; Registry information for add/remove programs
  WriteRegStr HKCU "$GogokeUninstallKey" "DisplayName" "${PRODUCTNAME}"
  WriteRegStr HKCU "$GogokeUninstallKey" "DisplayIcon" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
  WriteRegStr HKCU "$GogokeUninstallKey" "DisplayVersion" "$GogokeVersion"
  WriteRegStr HKCU "$GogokeUninstallKey" "Publisher" "${MANUFACTURER}"
  WriteRegStr HKCU "$GogokeUninstallKey" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "$GogokeUninstallKey" "InstallDomain" "$GogokeReceiptDomain"
  WriteRegStr HKCU "$GogokeUninstallKey" "InstallInstanceId" "$GogokeInstallInstanceId"
  WriteRegStr HKCU "$GogokeUninstallKey" "UninstallString" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --uninstall"
  WriteRegStr HKCU "$GogokeUninstallKey" "QuietUninstallString" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --uninstall --quiet"
  WriteRegDWORD HKCU "$GogokeUninstallKey" "NoModify" "1"
  WriteRegDWORD HKCU "$GogokeUninstallKey" "NoRepair" "1"
  IfErrors install_registration_failed
  ReadRegStr $R0 HKCU "$GogokeUninstallKey" "InstallLocation"
  StrCmp $R0 $INSTDIR 0 install_registration_failed
  ReadRegStr $R0 HKCU "$GogokeUninstallKey" "DisplayVersion"
  StrCmp $R0 $GogokeVersion 0 install_registration_failed
  ReadRegStr $R0 HKCU "$GogokeUninstallKey" "InstallDomain"
  StrCmp $R0 $GogokeReceiptDomain 0 install_registration_failed
  ReadRegStr $R0 HKCU "$GogokeUninstallKey" "InstallInstanceId"
  StrCmp $R0 $GogokeInstallInstanceId 0 install_registration_failed
  ReadRegStr $R0 HKCU "$GogokeUninstallKey" "UninstallString"
  StrCmp $R0 "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --uninstall" 0 install_registration_failed
  Goto install_registration_verified
install_registration_failed:
  Abort "The Gogoke install registration could not be written and read back."
install_registration_verified:

  ${GetSize} "$INSTDIR" "/M=uninstall.exe /S=0K /G=0" $0 $1 $2
  IntOp $0 $0 + ${ESTIMATEDSIZE}
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "$GogokeUninstallKey" "EstimatedSize" "$0"

  !if "${HOMEPAGE}" != ""
    WriteRegStr HKCU "$GogokeUninstallKey" "URLInfoAbout" "${HOMEPAGE}"
    WriteRegStr HKCU "$GogokeUninstallKey" "URLUpdateInfo" "${HOMEPAGE}"
    WriteRegStr HKCU "$GogokeUninstallKey" "HelpLink" "${HOMEPAGE}"
  !endif

  ; Create start menu shortcut
  !insertmacro MUI_STARTMENU_WRITE_BEGIN Application
    Call CreateOrUpdateStartMenuShortcut
  !insertmacro MUI_STARTMENU_WRITE_END

  ; Rebind an already owned desktop link to this install instance even when
  ; the optional finish-page creation checkbox is left unchecked.
  ${If} $GogokeInstallDomain != "CI_CANDIDATE_RESOURCE"
    StrCpy $GogokeShortcutKind "desktop"
    StrCpy $GogokeShortcutFolder ""
    StrCpy $GogokeShortcutMode "adopt"
    Call GogokeWriteOwnedShortcut
  ${EndIf}

  ; Create desktop shortcut for silent and passive installers
  ; because finish page will be skipped
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    Call CreateOrUpdateDesktopShortcut
  ${EndIf}

  !ifmacrodef NSIS_HOOK_POSTINSTALL
    !insertmacro NSIS_HOOK_POSTINSTALL
  !endif

  ; Auto close this page for passive mode
  ${If} $PassiveMode = 1
    SetAutoClose true
  ${EndIf}
SectionEnd

Function .onInstSuccess
  ; Finish-page shortcut creation still needs this exact lifecycle lock.
  ; .onGUIEnd releases it after the finish page and optional shortcut action.
  ; Check for `/R` flag only in silent and passive installers because
  ; GUI installer has a toggle for the user to (re)start the app
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    ${GetOptions} $CMDLINE "/R" $R0
    ${IfNot} ${Errors}
      ${GetOptions} $CMDLINE "/ARGS" $R0
      nsis_tauri_utils::RunAsUser "$INSTDIR\${MAINBINARYNAME}.exe" "$R0"
    ${EndIf}
  ${EndIf}
FunctionEnd

Function Skip
  Abort
FunctionEnd

Function SkipIfPassive
  ${IfThen} $PassiveMode = 1 ${|} Abort ${|}
FunctionEnd

Function CreateOrUpdateStartMenuShortcut
  ${If} $GogokeInstallDomain == "CI_CANDIDATE_RESOURCE"
    Return
  ${EndIf}
  StrCpy $GogokeShortcutKind "start"
  StrCpy $GogokeShortcutFolder "$AppStartMenuFolder"
  StrCpy $GogokeShortcutMode "create"
  ${If} $UpdateMode = 1
  ${OrIf} $NoShortcutMode = 1
    StrCpy $GogokeShortcutMode "adopt"
  ${EndIf}
  Call GogokeWriteOwnedShortcut
FunctionEnd

Function CreateOrUpdateDesktopShortcut
  ${If} $GogokeInstallDomain == "CI_CANDIDATE_RESOURCE"
    Return
  ${EndIf}
  StrCpy $GogokeShortcutKind "desktop"
  StrCpy $GogokeShortcutFolder ""
  StrCpy $GogokeShortcutMode "create"
  ${If} $UpdateMode = 1
  ${OrIf} $NoShortcutMode = 1
    StrCpy $GogokeShortcutMode "adopt"
  ${EndIf}
  Call GogokeWriteOwnedShortcut
FunctionEnd

Function GogokeWriteOwnedShortcut
  ; ExecWait disables inheritance. STARTUPINFOEX passes only the installer
  ; lifecycle handle, so the installed writer can verify the exact custody.
  StrCpy $GogokeShortcutDuplicate ""
  StrCpy $GogokeShortcutHandleList ""
  StrCpy $GogokeShortcutAttributes ""
  StrCpy $GogokeShortcutAttributesInitialized 0
  StrCpy $GogokeShortcutStartup ""
  StrCpy $GogokeShortcutProcessInfo ""
  StrCpy $GogokeShortcutCommand ""
  StrCpy $GogokeShortcutProcess ""
  StrCpy $GogokeShortcutThread ""
  System::Call 'kernel32::GetCurrentProcess() p .r0'
  System::Call 'kernel32::DuplicateHandle(p $0, p $GogokeLifecycleLockHandle, p $0, *p .r1, i 0, i 1, i 2) i .r2'
  ${If} $2 == 0
    Goto gogoke_shortcut_failed
  ${EndIf}
  StrCpy $GogokeShortcutDuplicate $1
  System::Alloc ${NSIS_PTR_SIZE}
  Pop $GogokeShortcutHandleList
  ${If} $GogokeShortcutHandleList == 0
    Goto gogoke_shortcut_failed
  ${EndIf}
  System::Call '*$GogokeShortcutHandleList(p $GogokeShortcutDuplicate)'
  System::Call 'kernel32::InitializeProcThreadAttributeList(p 0, i 1, i 0, *p .r0) i .r1'
  StrCpy $GogokeShortcutAttributeSize $0
  ${If} $GogokeShortcutAttributeSize <= 0
    Goto gogoke_shortcut_failed
  ${EndIf}
  ${If} $GogokeShortcutAttributeSize > 4096
    Goto gogoke_shortcut_failed
  ${EndIf}
  System::Alloc $GogokeShortcutAttributeSize
  Pop $GogokeShortcutAttributes
  ${If} $GogokeShortcutAttributes == 0
    Goto gogoke_shortcut_failed
  ${EndIf}
  System::Call 'kernel32::InitializeProcThreadAttributeList(p $GogokeShortcutAttributes, i 1, i 0, *p r0) i .r1'
  ${If} $1 == 0
    Goto gogoke_shortcut_failed
  ${EndIf}
  StrCpy $GogokeShortcutAttributesInitialized 1
  System::Call 'kernel32::UpdateProcThreadAttribute(p $GogokeShortcutAttributes, i 0, p 0x00020002, p $GogokeShortcutHandleList, p ${NSIS_PTR_SIZE}, p 0, p 0) i .r1'
  ${If} $1 == 0
    Goto gogoke_shortcut_failed
  ${EndIf}
  ; VirtualAlloc returns zeroed storage for STARTUPINFOEX and the mutable
  ; command line. Its native pointer width matches this NSIS process.
  System::Call 'kernel32::VirtualAlloc(p 0, p ${GOGOKE_STARTUP_EX_SIZE}, i 0x3000, i 0x04) p .r0'
  StrCpy $GogokeShortcutStartup $0
  System::Call 'kernel32::VirtualAlloc(p 0, p ${GOGOKE_PROCESS_INFO_SIZE}, i 0x3000, i 0x04) p .r0'
  StrCpy $GogokeShortcutProcessInfo $0
  System::Call 'kernel32::VirtualAlloc(p 0, p 8192, i 0x3000, i 0x04) p .r0'
  StrCpy $GogokeShortcutCommand $0
  ${If} $GogokeShortcutStartup == 0
  ${OrIf} $GogokeShortcutProcessInfo == 0
  ${OrIf} $GogokeShortcutCommand == 0
    Goto gogoke_shortcut_failed
  ${EndIf}
  System::Call '*$GogokeShortcutStartup(i ${GOGOKE_STARTUP_EX_SIZE})'
  IntOp $0 $GogokeShortcutStartup + ${GOGOKE_STARTUP_EX_ATTRIBUTES_OFFSET}
  System::Call '*$0(p $GogokeShortcutAttributes)'
  StrCpy $9 '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --gogoke-install-shortcut $GogokeShortcutKind $\"$GogokeShortcutFolder$\" $GogokeShortcutDuplicate $GogokeShortcutMode'
  StrLen $0 $9
  ${If} $0 > 4000
    Goto gogoke_shortcut_failed
  ${EndIf}
  System::Call 'kernel32::lstrcpyW(p $GogokeShortcutCommand, w r9) p'
  System::Call 'kernel32::CreateProcessW(w "$INSTDIR\${MAINBINARYNAME}.exe", p $GogokeShortcutCommand, p 0, p 0, i 1, i 0x00080000, p 0, w "$INSTDIR", p $GogokeShortcutStartup, p $GogokeShortcutProcessInfo) i .r0'
  ${If} $0 == 0
    Goto gogoke_shortcut_failed
  ${EndIf}
  System::Call '*$GogokeShortcutProcessInfo(p .r0, p .r1)'
  StrCpy $GogokeShortcutProcess $0
  StrCpy $GogokeShortcutThread $1
  System::Call 'kernel32::WaitForSingleObject(p $GogokeShortcutProcess, i 120000) i .r0'
  ${If} $0 != 0
    Goto gogoke_shortcut_failed
  ${EndIf}
  System::Call 'kernel32::GetExitCodeProcess(p $GogokeShortcutProcess, *i .r0) i .r1'
  ${If} $1 == 0
    Goto gogoke_shortcut_failed
  ${EndIf}
  ${If} $0 != 0
    Goto gogoke_shortcut_failed
  ${EndIf}
  Call GogokeReleaseShortcutLauncher
  Return
gogoke_shortcut_failed:
  ; Closing a process handle does not stop the child. Confirm its exit before
  ; the installer releases its lifecycle lock and reports a settled failure.
  StrCpy $GogokeShortcutChildUnknown 0
  ${If} $GogokeShortcutProcess != ""
    System::Call 'kernel32::WaitForSingleObject(p $GogokeShortcutProcess, i 0) i .r0'
    ${If} $0 != 0
      System::Call 'kernel32::TerminateProcess(p $GogokeShortcutProcess, i 1) i .r0'
      System::Call 'kernel32::WaitForSingleObject(p $GogokeShortcutProcess, i 10000) i .r0'
      ${If} $0 != 0
        StrCpy $GogokeShortcutChildUnknown 1
      ${EndIf}
    ${EndIf}
  ${EndIf}
  Call GogokeReleaseShortcutLauncher
  ${If} $GogokeShortcutChildUnknown == 1
    Abort "Gogoke shortcut writer exit is unconfirmed; installation custody is unresolved."
  ${EndIf}
  Abort "Gogoke could not create or retain a provably owned shortcut."
FunctionEnd

Function GogokeReleaseShortcutLauncher
  ${If} $GogokeShortcutThread != ""
    System::Call 'kernel32::CloseHandle(p $GogokeShortcutThread)'
    StrCpy $GogokeShortcutThread ""
  ${EndIf}
  ${If} $GogokeShortcutProcess != ""
    System::Call 'kernel32::CloseHandle(p $GogokeShortcutProcess)'
    StrCpy $GogokeShortcutProcess ""
  ${EndIf}
  ${If} $GogokeShortcutDuplicate != ""
    System::Call 'kernel32::CloseHandle(p $GogokeShortcutDuplicate)'
    StrCpy $GogokeShortcutDuplicate ""
  ${EndIf}
  ${If} $GogokeShortcutAttributes != ""
    ${If} $GogokeShortcutAttributesInitialized == 1
      System::Call 'kernel32::DeleteProcThreadAttributeList(p $GogokeShortcutAttributes)'
    ${EndIf}
    System::Free $GogokeShortcutAttributes
    StrCpy $GogokeShortcutAttributes ""
  ${EndIf}
  ${If} $GogokeShortcutHandleList != ""
    System::Free $GogokeShortcutHandleList
    StrCpy $GogokeShortcutHandleList ""
  ${EndIf}
  ${If} $GogokeShortcutStartup != ""
    System::Call 'kernel32::VirtualFree(p $GogokeShortcutStartup, p 0, i 0x8000)'
    StrCpy $GogokeShortcutStartup ""
  ${EndIf}
  ${If} $GogokeShortcutProcessInfo != ""
    System::Call 'kernel32::VirtualFree(p $GogokeShortcutProcessInfo, p 0, i 0x8000)'
    StrCpy $GogokeShortcutProcessInfo ""
  ${EndIf}
  ${If} $GogokeShortcutCommand != ""
    System::Call 'kernel32::VirtualFree(p $GogokeShortcutCommand, p 0, i 0x8000)'
    StrCpy $GogokeShortcutCommand ""
  ${EndIf}
FunctionEnd
