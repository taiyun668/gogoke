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

Var PassiveMode
Var UpdateMode
Var NoShortcutMode
Var WixMode
Var OldMainBinaryName
Var GogokeInstallDomain
Var GogokeUninstallKey
Var GogokeProductKey
Var GogokeDefaultRoot
Var GogokeVersion
Var GogokeReceiptDomain
Var GogokeInstallInstanceId
Var GogokeLifecycleLockHandle

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
    StrCpy $GogokeProductKey "Software\gogoke\gogoke-candidate"
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
    StrCpy $GogokeProductKey "Software\gogoke\gogoke"
    StrCpy $GogokeDefaultRoot "$LOCALAPPDATA\gogoke"
    Return
  ${EndIf}
  Goto install_domain_selected
invalid_install_set:
  Abort "The installer sibling set must contain exactly one signed candidate or formal manifest pair."
install_domain_selected:
FunctionEnd

Function AcquireGogokeLifecycleLock
  ${GetParent} "$INSTDIR" $0
  System::Call 'kernel32::CreateFileW(w "$0\gogoke-install-lifecycle.lock", i 0xC0000000, i 0, p 0, i 4, i 0x80, p 0) p .r0'
  ${If} $0 == -1
    Abort "Another Gogoke install, update, or uninstall operation holds the lifecycle lock."
  ${EndIf}
  StrCpy $GogokeLifecycleLockHandle $0
FunctionEnd

Function ReleaseGogokeLifecycleLock
  ${If} $GogokeLifecycleLockHandle != ""
    System::Call 'kernel32::CloseHandle(p $GogokeLifecycleLockHandle)'
    StrCpy $GogokeLifecycleLockHandle ""
  ${EndIf}
FunctionEnd

Function .onGUIEnd
  Call ReleaseGogokeLifecycleLock
FunctionEnd


Section EarlyChecks
  Call AcquireGogokeLifecycleLock
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
  InitPluginsDir
  SetOutPath "$PLUGINSDIR\gogoke-preflight"
  File /oname=gogoke.exe "${MAINBINARYSRCPATH}"
  ExecWait '"$PLUGINSDIR\gogoke-preflight\gogoke.exe" "--gogoke-verify-install-set=$EXEDIR" "--gogoke-install-target=$INSTDIR"' $0
  ${If} $0 != 0
    Abort "The signed Gogoke installer sibling set could not be verified."
  ${EndIf}
SectionEnd

Section WebView2
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
    ; Webview2 installation
    ;
    ; Skip if updating
    ${If} $UpdateMode <> 1
      !if "${INSTALLWEBVIEW2MODE}" == "downloadBootstrapper"
        Delete "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        DetailPrint "$(webview2Downloading)"
        NSISdl::download "https://go.microsoft.com/fwlink/p/?LinkId=2124703" "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        Pop $0
        ${If} $0 == "success"
          DetailPrint "$(webview2DownloadSuccess)"
        ${Else}
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
        DetailPrint "$(installingWebview2)"
        ; $6 holds the path to the webview2 installer
        ExecWait "$6 ${WEBVIEW2INSTALLERARGS} /install" $1
        ${If} $1 = 0
          DetailPrint "$(webview2InstallSuccess)"
        ${Else}
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
SectionEnd

Section Install
  SetOutPath $INSTDIR

  !ifmacrodef NSIS_HOOK_PREINSTALL
    !insertmacro NSIS_HOOK_PREINSTALL
  !endif

  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"

  ; Copy main executable
  File "${MAINBINARYSRCPATH}"

  ; Copy resources
  {{#each resources_dirs}}
    CreateDirectory "$INSTDIR\\{{this}}"
  {{/each}}
  {{#each resources}}
    File /a "/oname={{this.[1]}}" "{{no-escape @key}}"
  {{/each}}

  ; Copy external binaries
  {{#each binaries}}
    File /a "/oname={{this}}" "{{no-escape @key}}"
  {{/each}}

  ; Copy the already-signed sibling set without embedding or rewriting its sidecars.
  ClearErrors
  CopyFiles /SILENT "$EXEDIR\gogoke-resources.windows.zip" "$INSTDIR\gogoke-resources.windows.zip"
  IfErrors install_sidecar_copy_failed
  CopyFiles /SILENT "$EXEDIR\resource-index.json" "$INSTDIR\resource-index.json"
  IfErrors install_sidecar_copy_failed
  ${If} $GogokeInstallDomain == "CI_CANDIDATE_RESOURCE"
    CopyFiles /SILENT "$EXEDIR\CANDIDATE-RESOURCES.windows" "$INSTDIR\CANDIDATE-RESOURCES.windows"
    IfErrors install_sidecar_copy_failed
    CopyFiles /SILENT "$EXEDIR\CANDIDATE-RESOURCES.windows.sig" "$INSTDIR\CANDIDATE-RESOURCES.windows.sig"
    IfErrors install_sidecar_copy_failed
  ${Else}
    CopyFiles /SILENT "$EXEDIR\SHA256SUMS.windows" "$INSTDIR\SHA256SUMS.windows"
    IfErrors install_sidecar_copy_failed
    CopyFiles /SILENT "$EXEDIR\SHA256SUMS.windows.sig" "$INSTDIR\SHA256SUMS.windows.sig"
    IfErrors install_sidecar_copy_failed
  ${EndIf}

  Delete "$INSTDIR\gogoke-install-receipt.ini"
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" "--gogoke-install-resources=$EXEDIR"' $0
  ${If} $0 != 0
    Delete "$INSTDIR\gogoke-install-receipt.ini"
    Abort "The signed Gogoke install set could not be verified and installed."
  ${EndIf}
  ReadINIStr $GogokeVersion "$INSTDIR\gogoke-install-receipt.ini" "Gogoke" "Version"
  ReadINIStr $GogokeReceiptDomain "$INSTDIR\gogoke-install-receipt.ini" "Gogoke" "Domain"
  ${If} $GogokeVersion == ""
    Delete "$INSTDIR\gogoke-install-receipt.ini"
    Abort "The Gogoke installer did not produce a verified version receipt."
  ${EndIf}
  ${If} $GogokeReceiptDomain != $GogokeInstallDomain
    Delete "$INSTDIR\gogoke-install-receipt.ini"
    Abort "The verified Gogoke set domain does not match the selected install root."
  ${EndIf}
  ClearErrors
  GetTempFileName $GogokeInstallInstanceId "$TEMP"
  IfErrors install_instance_id_failed
  Delete "$GogokeInstallInstanceId"
  Delete "$INSTDIR\gogoke-install-receipt.ini"
  Goto install_set_verified

install_sidecar_copy_failed:
  Abort "Could not copy the signed Gogoke install set into the installation root."
install_instance_id_failed:
  Delete "$INSTDIR\gogoke-install-receipt.ini"
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

  ; Activate the verified candidate or formal instance in its separate HKCU key.
  ClearErrors
  WriteRegStr HKCU "$GogokeProductKey" "" "$INSTDIR"

  !if "${INSTALLMODE}" == "both"
    ; Save install mode to be selected by default for the next installation such as updating
    ; or when uninstalling
    WriteRegStr SHCTX "$GogokeUninstallKey" $MultiUser.InstallMode 1
  !endif

  ; Retain legacy main-binary identity for shortcut migration. An unknown
  ; existing file is never removed by the new installer.
  ReadRegStr $OldMainBinaryName HKCU "$GogokeUninstallKey" "MainBinaryName"

  ; Save current MAINBINARYNAME for future updates
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
  Call ReleaseGogokeLifecycleLock
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
  ; We used to use product name as MAINBINARYNAME
  ; migrate old shortcuts to target the new MAINBINARYNAME
  StrCpy $R0 0

  !insertmacro IsShortcutTarget "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk" "$INSTDIR\$OldMainBinaryName"
  Pop $0
  ${If} $0 = 1
    !insertmacro SetShortcutTarget "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    StrCpy $R0 1
  ${EndIf}

  !insertmacro IsShortcutTarget "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\$OldMainBinaryName"
  Pop $0
  ${If} $0 = 1
    !insertmacro SetShortcutTarget "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    StrCpy $R0 1
  ${EndIf}

  ${If} $R0 = 1
    Return
  ${EndIf}

  ; Skip creating shortcut if in update mode or no shortcut mode
  ; but always create if migrating from wix
  ${If} $WixMode = 0
    ${If} $UpdateMode = 1
    ${OrIf} $NoShortcutMode = 1
      Return
    ${EndIf}
  ${EndIf}

  !if "${STARTMENUFOLDER}" != ""
    CreateDirectory "$SMPROGRAMS\$AppStartMenuFolder"
    CreateShortcut "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
  !else
    CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\${PRODUCTNAME}.lnk"
  !endif
FunctionEnd

Function CreateOrUpdateDesktopShortcut
  ; We used to use product name as MAINBINARYNAME
  ; migrate old shortcuts to target the new MAINBINARYNAME
  !insertmacro IsShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\$OldMainBinaryName"
  Pop $0
  ${If} $0 = 1
    !insertmacro SetShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    Return
  ${EndIf}

  ; Skip creating shortcut if in update mode or no shortcut mode
  ; but always create if migrating from wix
  ${If} $WixMode = 0
    ${If} $UpdateMode = 1
    ${OrIf} $NoShortcutMode = 1
      Return
    ${EndIf}
  ${EndIf}

  CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  !insertmacro SetLnkAppUserModelId "$DESKTOP\${PRODUCTNAME}.lnk"
FunctionEnd
