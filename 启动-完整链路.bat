@echo off
chcp 65001 >nul
title ALVR-HuaweiVR - 完整链路（激活IMU后让SteamVR画面独占眼镜）
echo ============================================
echo  SteamVR 串流 + 头部追踪 完整启动
echo ============================================
echo.

echo [1/5] 启动 PCCastScreen 激活 IMU/SLAM 硬件...
start "" "F:\PY\HwPCVR Assistant\pcCastScreen\PCCastScreen.exe"
timeout /t 10 /nobreak >nul

echo [2/5] 关闭 PCCastScreen（释放眼镜屏幕，避免它显示桌面盖住SteamVR画面）...
taskkill /IM PCCastScreen.exe /F >nul 2>&1
echo       已关闭，IMU/SLAM 保持激活

echo [3/5] 启动手柄+头姿桥接 slam_bridge（读VR6DOF SLAM，推9912头姿/9913手柄）...
start "" /D "F:\PY\VrGlass\PC\ALVR-HuaweiVR\产物\客户端-PC" "F:\PY\VrGlass\PC\ALVR-HuaweiVR\产物\客户端-PC\slam_bridge.exe"
timeout /t 3 /nobreak >nul

echo [4/5] 确认 ALVR 接收引擎与渲染器运行中...
echo       接收引擎: hvr-alvr-win-sink.exe
echo       眼镜渲染: renderer\HwVRDesktop.exe
echo       （若未运行，用计划任务 HVR-Sink / HVR-Render 启动）

echo [5/5] 完成。头显应显示 SteamVR 场景并随头部移动。
echo 查看日志: 产物\客户端-PC\sink_live.log
echo          产物\客户端-PC\renderer\desktop_mode.log
echo.
pause
