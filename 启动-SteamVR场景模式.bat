@echo off
chcp 65001 >nul
title ALVR-HuaweiVR - SteamVR场景/VR游戏模式
echo ============================================
echo  SteamVR 场景 / VR 游戏模式
echo  头显显示：SteamVR场景或VR游戏画面
echo ============================================
echo.

set "PROD=F:\PY\VrGlass\PC\ALVR-HuaweiVR\产物\客户端-PC"
set "RD=%PROD%\renderer"
set "VR=F:\PY\SteamVR2.18.4\SteamVR\bin\win64\vrmonitor.exe"
set "DASH=F:\PY\VrGlass\PC\HuaweiVRGlass-ALVR\build\end\server-v2\ALVR Dashboard.exe"
set "FFMPEG=D:\666\ffmpeg\bin\ffmpeg.exe"

echo [0/4] 确保配置为 SteamVR 场景模式 (cast=false, kill_steamvr=0)
python -c "import json;p=r'F:\PY\VrGlass\PC\HuaweiVRGlass-ALVR\build\end\server-v2\session.json';s=json.load(open(p,encoding='utf-8-sig'));s['openvr_config']['cast_mode']=False;s['session_settings']['headset']['cast']['enabled']=False;json.dump(s,open(p,'w',encoding='utf-8'),ensure_ascii=False,indent=2)"
python -c "p=r'%RD%\desktop_mode.ini';s=open(p,encoding='utf-8').read().replace('kill_steamvr=1','kill_steamvr=0');open(p,'w',encoding='utf-8').write(s)"

echo [1/4] 启动 SteamVR
start "" "%VR%"
timeout /t 25 /nobreak >nul

echo [2/4] 启动 ALVR Dashboard
start "" "%DASH%"
timeout /t 10 /nobreak >nul

echo [3/4] 启动眼镜渲染器（不杀SteamVR）
start "" "%RD%\HwVRDesktop.exe"
timeout /t 5 /nobreak >nul

echo [4/4] 启动接收核心
set "FFMPEG=%FFMPEG%"
start "" /D "%PROD%" "%PROD%\hvr-alvr-win-sink.exe" --config sink_config.json

echo.
echo 完成。戴头显查看 SteamVR 场景。
echo 查看渲染状态: type "%RD%\desktop_mode.log"
echo 退出：关闭各窗口或 taskkill HwVRDesktop.exe / hvr-alvr-win-sink.exe
pause
