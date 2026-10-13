@echo off
rem Builds dist\AppRemover.exe (needs Python 3.9+ from python.org). The exe asks for admin rights itself.
python -m pip install --upgrade pyinstaller || exit /b 1
python -m PyInstaller --onefile --noconsole --uac-admin --name AppRemover run.pyw || exit /b 1
echo.
echo Done: dist\AppRemover.exe
