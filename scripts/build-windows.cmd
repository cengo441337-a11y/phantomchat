@echo off
REM PhantomChat Windows build helper.
REM
REM Required environment variables (set before running):
REM
REM   PHANTOMCHAT_SIGNTOOL       Full path to signtool.exe (Windows SDK).
REM                              Example: C:\Program Files (x86)\Windows Kits\10\bin\10.0.19041.0\x64\signtool.exe
REM   PHANTOMCHAT_PFX_PATH       Full path to the .pfx code-signing certificate.
REM   PHANTOMCHAT_PFX_PASSWORD   Password for the .pfx (in cleartext).
REM
REM Per the Nexus rule (memory: "Nexus - never install on C:") all paths
REM here assume D: or E: layout. Adjust if your toolchain layout differs.
REM
REM Usage:
REM   scripts\build-windows.cmd

setlocal

if "%PHANTOMCHAT_SIGNTOOL%"=="" (
    echo ERROR: PHANTOMCHAT_SIGNTOOL is not set.
    echo Example: set PHANTOMCHAT_SIGNTOOL=C:\Program Files ^(x86^)\Windows Kits\10\bin\10.0.19041.0\x64\signtool.exe
    exit /b 1
)
if "%PHANTOMCHAT_PFX_PATH%"=="" (
    echo ERROR: PHANTOMCHAT_PFX_PATH is not set.
    exit /b 1
)
if "%PHANTOMCHAT_PFX_PASSWORD%"=="" (
    echo ERROR: PHANTOMCHAT_PFX_PASSWORD is not set.
    exit /b 1
)

if not exist "%PHANTOMCHAT_SIGNTOOL%" (
    echo ERROR: signtool not found at "%PHANTOMCHAT_SIGNTOOL%"
    exit /b 1
)
if not exist "%PHANTOMCHAT_PFX_PATH%" (
    echo ERROR: PFX not found at "%PHANTOMCHAT_PFX_PATH%"
    exit /b 1
)

echo === Building signed Windows bundles ===
echo Signtool: %PHANTOMCHAT_SIGNTOOL%
echo PFX:      %PHANTOMCHAT_PFX_PATH%
echo.

cd /d "%~dp0\..\desktop"
if errorlevel 1 (
    echo ERROR: cannot cd to desktop\
    exit /b 1
)

cargo tauri build %*
if errorlevel 1 (
    echo BUILD FAILED.
    exit /b 1
)

echo.
echo === Done. Artifacts (auto-signed by Tauri's signCommand): ===
for %%f in (
    "..\target\release\phantomchat_desktop.exe"
    "..\target\release\bundle\msi\PhantomChat_*_x64_en-US.msi"
    "..\target\release\bundle\nsis\PhantomChat_*_x64-setup.exe"
) do (
    if exist "%%~f" (
        echo   %%~f
    )
)

endlocal
