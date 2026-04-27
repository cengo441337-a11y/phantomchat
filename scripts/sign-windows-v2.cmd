@echo off
REM Tauri signCommand wrapper — v2 self-signed pilot cert (no password).
REM
REM This is the unattended companion to scripts\sign-windows.cmd:
REM   - sign-windows.cmd        reads PHANTOMCHAT_PFX_PATH + PHANTOMCHAT_PFX_PASSWORD
REM                             from env, fails fast if missing. Used for the
REM                             original pilot cert whose password is not on disk.
REM   - sign-windows-v2.cmd     hard-codes the v2 pilot cert path with empty
REM                             password, so `cargo tauri build` on Nexus signs
REM                             automatically without env-var setup. Lets the
REM                             tauri.conf.json signCommand stay enabled at all
REM                             times instead of being stripped per build.
REM
REM Cert gen: see scripts\sign-no-password.ps1 (one-shot, idempotent).
REM Cert pinned in CI verification: keys/phantomchat-pilot-cert-v2.cer
REM
REM Usage (invoked by Tauri via tauri.conf.json bundle.windows.signCommand):
REM   scripts\sign-windows-v2.cmd <file-to-sign>

setlocal EnableDelayedExpansion

if "%~1"=="" (
    echo ERROR [sign-windows-v2.cmd]: missing target file argument 1>&2
    exit /b 1
)

set "PFX=E:\phantomchat-pilot-cert-v2.pfx"
if not exist "%PFX%" (
    echo ERROR [sign-windows-v2.cmd]: v2 cert not found at %PFX% 1>&2
    echo   Generate it once with: scripts\sign-no-password.ps1 1>&2
    exit /b 1
)

REM Locate signtool.exe by walking the Windows-10-SDK install tree. cmd
REM `where` only finds it if the SDK's bin dir is on PATH (which on a
REM clean Nexus shell it isn't), so we fall back to a glob over
REM `C:\Program Files (x86)\Windows Kits\10\bin\<sdk-version>\x64\`.
REM `dir /b /od` orders by date so the newest SDK wins; pick the last
REM matching line via the FOR loop.
set "SIGNTOOL="
for /f "delims=" %%i in ('dir /b /od "C:\Program Files (x86)\Windows Kits\10\bin\*" 2^>nul') do (
    if exist "C:\Program Files (x86)\Windows Kits\10\bin\%%i\x64\signtool.exe" (
        set "SIGNTOOL=C:\Program Files (x86)\Windows Kits\10\bin\%%i\x64\signtool.exe"
    )
)
REM Fallback: try `where` in case the SDK bin IS on PATH.
if not defined SIGNTOOL (
    where signtool >nul 2>&1
    if not errorlevel 1 set "SIGNTOOL=signtool"
)
if not defined SIGNTOOL (
    echo ERROR [sign-windows-v2.cmd]: signtool.exe not found 1>&2
    echo   Install Windows 10 SDK ^(any version^) into the default location, 1>&2
    echo   or add the SDK bin/x64 dir to PATH. 1>&2
    exit /b 1
)

"%SIGNTOOL%" sign ^
    /f "%PFX%" ^
    /p "" ^
    /fd SHA256 ^
    /td SHA256 ^
    /tr http://timestamp.digicert.com ^
    /d "PhantomChat" ^
    /du "https://dc-infosec.de/phantomchat" ^
    "%~1"
