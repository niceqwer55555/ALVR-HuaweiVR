@echo off
chcp 65001 >nul
title ALVR-HuaweiVR - 桌面投屏(cast)模式
echo ============================================
echo  桌面投屏 (cast) 模式
echo  头显显示：PC 桌面（SBS 立体投屏）
echo  注意：此模式不需要 SteamVR，渲染器会杀掉它
echo ============================================
echo.

set "PROD=F:\PY\VrGlass\PC\ALVR-HuaweiVR\产物\客户端-PC"
set "RD=%PROD%\renderer"
set "DASH=F:\PY\VrGlass\PC\HuaweiVRGlass-ALVR\build\end\server-v2\ALVR Dashboard.exe"
set "FFMPEG=D:\666\ffmpeg\bin\ffmpeg.exe"

echo [0/4] 确保配置为 cast 模式 (cast=true, kill_steamvr=1)
python -c "import json;p=r'F:\PY\VrGlass\PC\HuaweiVRGlass-ALVR\build\end\server-v2\session.json';s=json.load(open(p,encoding='utf-8-sig'));s['openvr_config']['cast_mode']=True;s['session_settings']['headset']['cast']['enabled']=True;json.dump(s,open(p,'w',encoding='utf-8'),ensure_ascii=False,indent=2)"
python -c "p=r'%RD%\desktop_mode.ini';s=open(p,encoding='utf-8').read().replace('kill_steamvr=0','kill_steamvr=1');open(p,'w',encoding='utf-8').write(s)"
taskkill /F /IM vrserver.exe >nul 2>&1
taskkill /F /IM vrmonitor.exe >nul 2>&1

echo [1/4] 启动 ALVR Dashboard（cast 帧源）
start "" "%DASH%"
timeout /t 12 /nobreak >nul

echo [2/4] 启动眼镜渲染器（杀SteamVR独占面板）
start "" "%RD%\HwVRDesktop.exe"
timeout /t 5 /nobreak >nul

echo [3/4] 启动接收核心
set "FFMPEG=%FFMPEG%"
start "" /D "%PROD%" "%PROD%\hvr-alvr-win-sink.exe" --config sink_config.json

echo.
echo 完成。戴头显查看 PC 桌面投屏。
echo 查看渲染状态: type "%RD%\desktop_mode.log"
pause
