@echo off
rem Qamille: build Qamille.exe (desktop app, the UI embedded) in release,
rem assemble it in .\release, install into D:\Software\Qamille, clean what the build staged.
rem All the app's data is in data\ beside Qamille.exe: the install keeps it.
rem
rem The edition is decided at build time (spec A6). Without QAMILLE_SERVER: self-hosted, the server
rem address is written by hand. With it, the Professional edition bound to that server, installed
rem into D:\Software\Qamille-Professional, plus the per-user NSIS installer in release\installer:
rem   set QAMILLE_SERVER=https://qamille.example
rem   build-release.bat
rem The address must be https:// (or http://127.0.0.1 for a test): an invalid one fails the build.
rem
rem   build-release.bat                   build, .\release, install, clean
rem   build-release.bat --no-install      stop at .\release
rem   build-release.bat --target <dir>    install folder (default D:\Software\Qamille)
rem   build-release.bat --ci              npm ci even when node_modules is already there
rem   build-release.bat --jobs N          parallel Rust compilations (CARGO_BUILD_JOBS)
rem
rem The steps; the PROJECT parts are the settings below and the subroutines at the end.
rem   1. tools, version, npm dependencies in NPM_DIR (npm ci only when node_modules is missing, or --ci)
rem   2. build                                                          PROJECT :build
rem   3. .\release made from nothing: the programs and their files      PROJECT :assemble
rem   4. install: .\release is copied over D:\Software\Qamille, made if missing. What only the
rem      install holds (data, webview ...) stays as it is; nothing is backed up.
rem      release\installer\, where there is one, holds installers to hand out: it is not installed.
rem      While a program of the install folder is running, it waits for it to close.
rem   5. clean: what the build staged is removed, on success and on failure. PROJECT :clean_project
rem      Cargo's target\ stays (the build cache: the next build is incremental), node_modules stays.
rem
rem Errors always leave through a label: with setlocal on, "exit /b 1" inside a parenthesized
rem block returns 0 to whoever ran the script.

setlocal EnableExtensions
set "ROOT=%~dp0"
set "SELF=%~nx0"
set "RELEASE=%ROOT%release"
cd /d "%ROOT%" || goto :fail

rem ==== PROJECT: settings ========================================================================
set "APP=Qamille"
if defined QAMILLE_SERVER set "APP=Qamille-Professional"
rem The program that must be in .\release once it is assembled.
set "APP_EXE=Qamille.exe"
rem Tools beyond cargo, node, npm and git.
set "TOOLS="
rem The release builds run out of memory with several jobs: 1 unless CARGO_BUILD_JOBS or --jobs
rem say otherwise.
set "DEFAULT_JOBS=1"
rem The folder (relative to the repo) with package.json and node_modules.
set "NPM_DIR=desktop"
rem ===============================================================================================

set "INSTALL=D:\Software\%APP%"
set "DO_INSTALL=1"
set "NPM_CI=0"

:args
if "%~1"=="" goto :args_done
if /i "%~1"=="--no-install" (set "DO_INSTALL=0" & shift & goto :args)
if /i "%~1"=="--ci" (set "NPM_CI=1" & shift & goto :args)
if /i "%~1"=="--target" goto :arg_target
if /i "%~1"=="--jobs" goto :arg_jobs
goto :usage
:arg_target
if "%~2"=="" goto :usage
set "INSTALL=%~f2"
shift & shift & goto :args
:arg_jobs
if "%~2"=="" goto :usage
set "CARGO_BUILD_JOBS=%~2"
shift & shift & goto :args
:args_done
if "%INSTALL:~-1%"=="\" set "INSTALL=%INSTALL:~0,-1%"
if not defined CARGO_BUILD_JOBS if defined DEFAULT_JOBS set "CARGO_BUILD_JOBS=%DEFAULT_JOBS%"

rem --- 1. Tools and dependencies -----------------------------------------------------------------
echo.
echo [1/5] Tools and dependencies
for %%T in (cargo node npm git %TOOLS%) do (
  where %%T >nul 2>nul || (echo [ERROR] %%T not found in PATH. & goto :fail)
)
rem Tauri's build scripts compile the Windows resources with rc.exe (embed-resource). Its own Windows SDK
rem lookup can name an rc.exe that is not there, and only RC_<target> or RC come before it: unless
rem RC_x86_64_pc_windows_msvc is already set, point it at the rc.exe of the newest SDK that has one.
if not defined RC_x86_64_pc_windows_msvc call :find_rc
set "VERSION=?"
for /f "usebackq delims=" %%V in (`node -p "require('./%NPM_DIR:\=/%/package.json').version"`) do set "VERSION=%%V"
set "COMMIT=?"
for /f "usebackq delims=" %%C in (`git rev-parse --short HEAD`) do set "COMMIT=%%C"
set "DIRTY="
git diff --quiet HEAD -- 2>nul || set "DIRTY=, uncommitted changes"
echo     %APP% %VERSION%, commit %COMMIT%%DIRTY%
if defined CARGO_BUILD_JOBS echo     %CARGO_BUILD_JOBS% parallel Rust jobs
if "%DO_INSTALL%"=="1" echo     install into %INSTALL%
if not "%DO_INSTALL%"=="1" echo     no install: .\release only

if "%NPM_CI%"=="1" goto :npm_ci
if exist "%NPM_DIR%\node_modules\.package-lock.json" goto :npm_done
:npm_ci
pushd "%NPM_DIR%" || goto :fail
call npm ci
set "RC=%ERRORLEVEL%"
popd
if not "%RC%"=="0" goto :fail
:npm_done

rem --- 2. Build ----------------------------------------------------------------------------------
echo.
echo [2/5] Build
call :build
if errorlevel 1 goto :fail

rem --- 3. Release folder -------------------------------------------------------------------------
echo.
echo [3/5] %RELEASE%
if exist "%RELEASE%\" rmdir /s /q "%RELEASE%"
if exist "%RELEASE%\" goto :release_busy
call :assemble
if errorlevel 1 goto :fail
if not exist "%RELEASE%\%APP_EXE%" goto :release_missing

rem --- 4. Install --------------------------------------------------------------------------------
echo.
if "%DO_INSTALL%"=="1" goto :do_install
echo [4/5] Install skipped ^(--no-install^)
goto :installed
:do_install
echo [4/5] Install into %INSTALL%
call :install
if errorlevel 1 goto :fail
:installed

rem --- 5. Clean ----------------------------------------------------------------------------------
echo.
echo [5/5] Clean
call :clean_project

echo.
echo === Done: %APP% %VERSION% (%COMMIT%%DIRTY%)
echo     release    %RELEASE%
if "%DO_INSTALL%"=="1" echo     installed  %INSTALL%\%APP_EXE%
call :pause_if_clicked
endlocal
exit /b 0

rem --- Common subroutines ------------------------------------------------------------------------

:install
if not exist "%INSTALL%\" mkdir "%INSTALL%" || goto :sub_fail
set "SHOWN="
:install_wait
set "RUNNING=0"
for /f "usebackq delims=" %%N in (`powershell -NoProfile -NonInteractive -Command "$d = [IO.Path]::GetFullPath('%INSTALL%') + '\'; @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($d, [StringComparison]::OrdinalIgnoreCase) }).Count"`) do set "RUNNING=%%N"
if "%RUNNING%"=="0" goto :install_copy
if not defined SHOWN echo     %APP% is running from %INSTALL%: close it, the install goes on by itself.
set "SHOWN=1"
ping -n 4 127.0.0.1 >nul
goto :install_wait
:install_copy
robocopy "%RELEASE%" "%INSTALL%" /E /XD "%RELEASE%\installer" /R:10 /W:2 /NFL /NDL /NJH /NP
if errorlevel 8 (echo [ERROR] copy into %INSTALL% failed: a file may still be in use. & goto :sub_fail)
exit /b 0

:find_rc
set "KITS=%ProgramFiles(x86)%\Windows Kits\10\bin"
set "RC_DIR="
for /f "delims=" %%D in ('dir /b /ad /o-n "%KITS%\10.*" 2^>nul') do if not defined RC_DIR if exist "%KITS%\%%D\x64\rc.exe" set "RC_DIR=%KITS%\%%D\x64"
if not defined RC_DIR echo     rc.exe not found in a Windows SDK: Tauri builds that recompile their resources will fail.
if defined RC_DIR set "RC_x86_64_pc_windows_msvc=%RC_DIR%\rc.exe"
if defined RC_DIR echo     rc.exe: %RC_DIR%\rc.exe
exit /b 0

:pause_if_clicked
rem Started with a double-click from Explorer: keep the window open to read the result. Never in CI.
if defined CI exit /b 0
echo %CMDCMDLINE% | "%SystemRoot%\System32\find.exe" /i "%SELF%" >nul && pause
exit /b 0

:sub_fail
exit /b 1

:release_busy
echo [ERROR] %RELEASE% could not be removed: a file in it is in use.
goto :fail

:release_missing
echo [ERROR] %RELEASE%\%APP_EXE% was not produced.
goto :fail

:usage
echo [ERROR] invalid arguments: %*
echo Usage: %SELF% [--no-install] [--target ^<folder^>] [--ci] [--jobs N]
call :pause_if_clicked
endlocal
exit /b 1

:fail
echo.
echo [ERROR] %APP% build-release stopped: read the error above.
call :clean_project
call :pause_if_clicked
endlocal
exit /b 1

rem ==== PROJECT: build, release folder, clean ===================================================
rem Each subroutine returns 0 when done; errors go to :sub_fail. The working folder is the repo.

:build
rem One cargo build folder for the app: app\target (desktop\.cargo\config.toml).
set "CARGO_TARGET_DIR=%ROOT%app\target"
set "QAMILLE_PRESET_SERVER="
if defined QAMILLE_SERVER set "QAMILLE_PRESET_SERVER=%QAMILLE_SERVER%"
if defined QAMILLE_SERVER echo     Professional edition, server %QAMILLE_SERVER%
if not defined QAMILLE_SERVER echo     self-hosted edition
echo     interface ^(npm run build^)
pushd "desktop" || goto :sub_fail
call npm run build
set "RC=%ERRORLEVEL%"
popd
if not "%RC%"=="0" goto :sub_fail
rem Always --features custom-protocol: without it the exe loads http://127.0.0.1:1420 (AGENTS.md).
echo     Qamille.exe ^(qamille-desktop, release, custom-protocol^)
cargo build --manifest-path desktop\src-tauri\Cargo.toml --release --features custom-protocol --locked || goto :sub_fail
if not defined QAMILLE_SERVER exit /b 0
echo     per-user NSIS installer of the Professional edition
if exist "%CARGO_TARGET_DIR%\release\bundle\nsis\" del /q "%CARGO_TARGET_DIR%\release\bundle\nsis\*.exe" 2>nul
rem With the updater's signing key (TAURI_SIGNING_PRIVATE_KEY, and its _PASSWORD) Tauri also signs the
rem installer for the updater (<installer>.sig beside it, docs/edizioni.md). Without it the command is the
rem same as before, byte for byte (native-feel check 27).
set "UPDATER_CONFIG="
if defined TAURI_SIGNING_PRIVATE_KEY set "UPDATER_CONFIG= --config src-tauri/tauri.updater.conf.json"
if defined TAURI_SIGNING_PRIVATE_KEY echo     updater artifacts ^(TAURI_SIGNING_PRIVATE_KEY^)
pushd "desktop" || goto :sub_fail
call npm run tauri build -- --bundles nsis --config src-tauri/tauri.professional.conf.json%UPDATER_CONFIG% --features custom-protocol
set "RC=%ERRORLEVEL%"
popd
if not "%RC%"=="0" goto :sub_fail
exit /b 0

:assemble
set "REL=%ROOT%app\target\release"
mkdir "%RELEASE%" || goto :sub_fail
copy /y "%REL%\qamille-desktop.exe" "%RELEASE%\Qamille.exe" >nul || goto :sub_fail
copy /y "docs\portable.md" "%RELEASE%\LEGGIMI.md" >nul || goto :sub_fail
if exist "%REL%\*.dll" copy /y "%REL%\*.dll" "%RELEASE%\" >nul || goto :sub_fail
if not defined QAMILLE_SERVER goto :assemble_done
set "QAMILLE_VERSION="
for /f "usebackq delims=" %%V in (`node -p "require('./desktop/src-tauri/tauri.conf.json').version"`) do set "QAMILLE_VERSION=%%V"
set "SETUP="
for %%F in ("%REL%\bundle\nsis\*.exe") do set "SETUP=%%~F"
if not defined SETUP (echo [ERROR] no installer in %REL%\bundle\nsis. & goto :sub_fail)
mkdir "%RELEASE%\installer" || goto :sub_fail
copy /y "%SETUP%" "%RELEASE%\installer\Qamille-Professional-Setup-%QAMILLE_VERSION%.exe" >nul || goto :sub_fail
:assemble_done
for %%F in ("%RELEASE%\*") do echo     %%~nxF  %%~zF bytes
if exist "%RELEASE%\installer\" for %%F in ("%RELEASE%\installer\*") do echo     installer\%%~nxF  %%~zF bytes
exit /b 0

:clean_project
rem The build stages nothing outside cargo's app\target and .\release.
rem The server no longer travels with the app: an old install still has its exe and robocopy
rem does not delete it.
if "%DO_INSTALL%"=="1" if exist "%INSTALL%\qamille-server.exe" del /f /q "%INSTALL%\qamille-server.exe"
exit /b 0
