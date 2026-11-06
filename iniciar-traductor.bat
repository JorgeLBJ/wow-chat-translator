@echo off
rem Starts the WoW Chat Translator overlay. It closes itself when WoW closes.
cd /d "%~dp0app"
start "" "node_modules\electron\dist\electron.exe" .
