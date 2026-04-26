@echo off
REM Tauri signCommand wrapper — Tauri's shlex parser does not expand %ENV%
REM in args, so this batch file does it for us. Reads PHANTOMCHAT_PFX_PATH /
REM PHANTOMCHAT_PFX_PASSWORD from env and signs the file Tauri passes as %1.
REM
REM Tauri invokes:  scripts\sign-windows.cmd <file-to-sign>

if "%PHANTOMCHAT_PFX_PATH%"=="" (
    echo ERROR [sign-windows.cmd]: PHANTOMCHAT_PFX_PATH not set 1>&2
    exit /b 1
)
if "%PHANTOMCHAT_PFX_PASSWORD%"=="" (
    echo ERROR [sign-windows.cmd]: PHANTOMCHAT_PFX_PASSWORD not set 1>&2
    exit /b 1
)
if "%~1"=="" (
    echo ERROR [sign-windows.cmd]: missing target file argument 1>&2
    exit /b 1
)

signtool sign ^
    /f "%PHANTOMCHAT_PFX_PATH%" ^
    /p "%PHANTOMCHAT_PFX_PASSWORD%" ^
    /fd SHA256 ^
    /td SHA256 ^
    /tr http://timestamp.digicert.com ^
    /d "PhantomChat" ^
    /du "https://dc-infosec.de/phantomchat" ^
    "%~1"

exit /b %ERRORLEVEL%
