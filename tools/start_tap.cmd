@echo off
rem Attaches the passive serial recorder to the running (elevated) Hytera CPS.
rem Run as administrator. Stop it by creating tools\tap.stop or closing this window.
"%~dp0..\.venv\Scripts\python.exe" "%~dp0tap_runner.py"
pause
