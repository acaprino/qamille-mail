; Hooks of the Professional per-user installer. tauri.professional.conf.json names this file in
; bundle > windows > nsis > installerHooks and Tauri's installer.nsi includes it; it runs the
; NSIS_HOOK_* macros below at the matching points of the install and uninstall sections.
;
; The uninstaller also runs, with /UPDATE, before an update replaces the files. $UpdateMode is 1 then,
; and nothing the user keeps across versions may be removed.

!macro NSIS_HOOK_PREUNINSTALL
  Push $0

  ; End the headless workers (Qamille --worker) started from this install folder, so that no process
  ; holds the program open while it is removed. A worker is matched on the full path of the
  ; executable and on --worker as a whole argument, never on the image name: another Qamille of the
  ; user (a portable copy, the window itself) is left alone. The path travels in the environment, so a
  ; quote or an apostrophe in the folder name cannot end up in the command. If PowerShell fails, the
  ; Restart Manager step that follows in the uninstaller still asks for what remains to be closed.
  System::Call `Kernel32::SetEnvironmentVariable(t "QAMILLE_EXE", t "$INSTDIR\${MAINBINARYNAME}.exe")`
  nsExec::Exec /TIMEOUT=20000 `"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -Command "Get-CimInstance Win32_Process | Where-Object { $$_.ExecutablePath -eq $$env:QAMILLE_EXE -and $$_.CommandLine -match '(^|\s)--worker(\s|$$)' } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force }"`
  Pop $0

  ; Remove the file that starts the worker at sign-in (the same path as startup_launcher() in
  ; app/src/infrastructure/mcp_config.rs, which also reads %APPDATA%): left behind, it makes Windows
  ; Script Host report an error at every sign-in. An update keeps it, as the template keeps the HKCU
  ; Run value: the worker must still start after the new version is in place.
  ${If} $UpdateMode <> 1
    ReadEnvStr $0 APPDATA
    ${If} $0 != ""
      Delete "$0\Microsoft\Windows\Start Menu\Programs\Startup\Qamille-worker.vbs"
    ${EndIf}
  ${EndIf}

  Pop $0
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Qamille keeps its data and credentials.key in the folder "data" beside the program
  ; (data_dir() in app/src/infrastructure/legacy_store.rs). The template's "Delete app data" box only
  ; empties the folders named after the bundle identifier under %APPDATA% and %LOCALAPPDATA%, where
  ; Qamille writes nothing, so the box is acted on here. Nothing is removed unless it is ticked, and
  ; never on an update.
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    RMDir /r "$INSTDIR\data"
    ; The template's RMDir "$INSTDIR" came before this, while data was still there. Only an empty
    ; install folder goes: a folder the user chose and shares with other files stays as it is.
    RMDir "$INSTDIR"
  ${EndIf}
!macroend
