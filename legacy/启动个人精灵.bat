@echo off
title 个人精灵
echo 正在启动个人精灵...
cd /d %~dp0
where python >nul 2>nul
if %errorlevel%==0 (
    pythonw personal_sprite.py
) else (
    echo 未找到 Python，请先安装: https://python.org
    pause
)
