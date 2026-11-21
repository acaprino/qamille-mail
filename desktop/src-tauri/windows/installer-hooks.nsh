; Hooks of the Professional per-user installer. tauri.professional.conf.json names this file in
; bundle > windows > nsis > installerHooks and Tauri's installer.nsi includes it; it runs the
; NSIS_HOOK_* macros below at the matching points of the install and uninstall sections.
;
; The uninstaller runs in three ways. A plain uninstall. An update the installer was started for with
; /UPDATE, which passes /UPDATE on: $UpdateMode is 1. And the default choice of the reinstall page for a
; manual upgrade, "uninstall before installing": the same uninstaller, with its confirm page, but
; without /UPDATE ($UpdateMode is empty) and about to be followed by the new version. So
; $UpdateMode <> 1 does not mean that Qamille is going away: what an uninstall removes and an upgrade
; must keep is also put back by the app at its next start (put_back_after_upgrade in
; desktop/src-tauri/src/background.rs, for the start at sign-in). The keys that let Windows offer Qamille
; for mailto: links are written again by every install, so an upgrade of either kind ends with them.

!macro NSIS_HOOK_PREUNINSTALL
  Push $0

  ; End the headless workers (Qamille --worker) started from this install folder, so that no process
  ; holds the program open while it is removed. A worker is matched on the full path of the
  ; executable and on --worker as a whole argument, never on the image name: another Qamille of the
  ; user (a portable copy, the window itself) is left alone. The path travels in the environment, so a
  ; quote or an apostrophe in the folder name cannot end up in the command. If PowerShell fails, the
  ; Restart Manager step that follows in the uninstaller still asks for what remains to be closed.
  ; Nothing is deleted here: the user can still cancel at that step and keep Qamille installed.
  System::Call `Kernel32::SetEnvironmentVariable(t "QAMILLE_EXE", t "$INSTDIR\${MAINBINARYNAME}.exe")`
  nsExec::Exec /TIMEOUT=20000 `"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -Command "Get-CimInstance Win32_Process | Where-Object { $$_.ExecutablePath -eq $$env:QAMILLE_EXE -and $$_.CommandLine -match '(^|\s)--worker(\s|$$)' } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force }"`
  ; The first Pop takes the result of nsExec, the second gives $0 back.
  Pop $0
  Pop $0
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Push $0

  ; Remove what starts Qamille at sign-in, so nothing starts or reports an error once it is gone. This
  ; runs only once the uninstall went through, never before the "Qamille is running" step, where it can
  ; still be cancelled. An /UPDATE keeps it; a manual upgrade removes it too, and the new version puts
  ; the entry back at its first start if it was on (see the top of this file).
  ; - The Run value that Settings > General writes for the installed Professional (its fixed name comes
  ;   from autostart_entry_name in desktop/src-tauri/src/background_rules.rs), and its twin where Task
  ;   Manager keeps its switch. Only HKCU: the app never writes the machine-wide entry.
  ; - The worker's startup file of the versions before the Run value (startup_launcher() in
  ;   app/src/infrastructure/mcp_config.rs, which also reads %APPDATA%): left behind, it makes Windows
  ;   Script Host report an error at every sign-in. The app migrates it when it starts; an install that
  ;   never started this version may still have it.
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Qamille"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "Qamille"
    ReadEnvStr $0 APPDATA
    ${If} $0 != ""
      Delete "$0\Microsoft\Windows\Start Menu\Programs\Startup\Qamille-worker.vbs"
    ${EndIf}
    ; The identity of the Windows notifications (desktop/src-tauri/src/notifications.rs): the app
    ; registered it under HKCU\Software\Classes\AppUserModelId\<identifier> and wrote the identifier in
    ; data\notification-aumid.txt, read here before the data folder may go below. Only an identifier of
    ; Qamille (com.qamille.desktop...) is removed. A manual upgrade removes it too; the new version
    ; registers it again at its first start while notifications are on.
    Push $1
    Push $2
    ClearErrors
    FileOpen $0 "$INSTDIR\data\notification-aumid.txt" r
    ${IfNot} ${Errors}
      FileRead $0 $1
      FileClose $0
      StrCpy $2 $1 19
      ${If} $2 == "com.qamille.desktop"
        DeleteRegKey HKCU "Software\Classes\AppUserModelId\$1"
      ${EndIf}
    ${EndIf}
    Pop $2
    Pop $1
  ${EndIf}

  ; Qamille keeps its data and credentials.key in the folder "data" beside the program
  ; (data_dir() in app/src/infrastructure/legacy_store.rs). The template's "Delete app data" box only
  ; empties the folders named after the bundle identifier under %APPDATA% and %LOCALAPPDATA%, where
  ; Qamille writes nothing, so the box is acted on here. Nothing is removed unless it is ticked, and
  ; never on an /UPDATE; an empty $INSTDIR would turn the path into \data.
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
  ${AndIf} $INSTDIR != ""
    RMDir /r "$INSTDIR\data"
    ; The template's RMDir "$INSTDIR" came before this, while data was still there. Only an empty
    ; install folder goes: a folder the user chose and shares with other files stays as it is.
    RMDir "$INSTDIR"
  ${EndIf}

  ; Windows no longer offers Qamille for mailto: links: the RegisteredApplications value, the ProgID and
  ; the Capabilities that NSIS_HOOK_POSTINSTALL wrote, the names of handler_keys(true, ...) in
  ; desktop/src-tauri/src/mail_handler.rs. Windows asks which app to use at the next link if Qamille was
  ; the one chosen (its choice is not Qamille's to write or remove). Qamille's key goes only when nothing
  ; else is in it. An /UPDATE keeps them; a manual upgrade removes them and its install writes them again.
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\RegisteredApplications" "Qamille"
    DeleteRegKey HKCU "Software\Classes\Qamille.Url.mailto"
    DeleteRegKey HKCU "Software\Qamille\MailLinks\Installed"
    DeleteRegKey /ifempty HKCU "Software\Qamille\MailLinks"
    DeleteRegKey /ifempty HKCU "Software\Qamille"
    System::Call "shell32::SHChangeNotify(i,i,i,i) (0x08000000, 0x1000, 0, 0)"
  ${EndIf}

  Pop $0
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Windows lets no program make itself the default mail app: the installed Professional only says it
  ; can open mailto: links, for the current user, and the person chooses it in Settings > Default apps
  ; (Settings > General > Windows in Qamille opens that page). These are exactly the values entries()
  ; writes in desktop/src-tauri/src/mail_handler.rs, with the fixed names of handler_keys(true, ...) and
  ; in the same order (native-feel check 26): the ProgID, whose command is the executable and the link in
  ; quotes, its Capabilities, then the RegisteredApplications value that points to them. Never the
  ; mailto key of the scheme itself, nor HKLM.
  WriteRegStr HKCU "Software\Classes\Qamille.Url.mailto" "" "Qamille"
  WriteRegStr HKCU "Software\Classes\Qamille.Url.mailto\DefaultIcon" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
  WriteRegStr HKCU "Software\Classes\Qamille.Url.mailto\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
  WriteRegStr HKCU "Software\Qamille\MailLinks\Installed\Capabilities" "ApplicationName" "Qamille"
  WriteRegStr HKCU "Software\Qamille\MailLinks\Installed\Capabilities" "ApplicationDescription" "Qamille"
  WriteRegStr HKCU "Software\Qamille\MailLinks\Installed\Capabilities" "ApplicationIcon" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
  WriteRegStr HKCU "Software\Qamille\MailLinks\Installed\Capabilities\URLAssociations" "mailto" "Qamille.Url.mailto"
  WriteRegStr HKCU "Software\RegisteredApplications" "Qamille" "Software\Qamille\MailLinks\Installed\Capabilities"
  ; SHCNE_ASSOCCHANGED with SHCNF_FLUSH, as the template's UPDATEFILEASSOC: Explorer and Settings read
  ; the associations again.
  System::Call "shell32::SHChangeNotify(i,i,i,i) (0x08000000, 0x1000, 0, 0)"
!macroend
