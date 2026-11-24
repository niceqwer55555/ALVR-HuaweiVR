package com.generic.alvr;

import android.app.Activity;
import android.content.Context;
import android.hardware.Sensor;
import android.hardware.SensorEvent;
import android.hardware.SensorEventListener;
import android.hardware.SensorManager;
import android.os.BatteryManager;
import android.media.MediaCodecInfo;
import android.media.MediaCodecList;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.view.MotionEvent;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.WindowManager;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.TextView;
import android.graphics.Color;

import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.util.Locale;

/**
 * 通用 ALVR OpenXR 接收端 Android Host —— 零华为依赖。
 *
 * 与参考仓库对比：
 *   - 不使用 com.huawei.* 权限 / HuaweiVrActivity / libvrpose.so / libxr_loader.so
 *   - 姿态源：手机 IMU（TYPE_ROTATION_VECTOR/GYROSCOPE/ACCELEROMETER）本地自算，
 *             同时监听 localhost UDP 47110 接收外部 6DoF 姿态（可插拔）
 *   - 手柄：触摸区虚拟手柄 + 蓝牙手柄按键映射（3DoF/6DoF 由 native 聚合）
 *   - 解码器：仅接受高通组件（c2.qti 及 OMX.qcom 系列），禁止软解
 *
 * 真正的 OpenXR 会话与解码渲染由 native 库 libalvr_client_openxr.so +
 * libgeneric_alvr_xr.so（通用 runtime）承担；本 Activity 只负责平台 I/O。
 */
public class AlvrActivity extends Activity implements SensorEventListener, SurfaceHolder.Callback {

    static { System.loadLibrary("generic_alvr_bridge"); }

    // ---- native 桥（见 jni_bridge.c）----
    private static native void nativeInit(int viewW, int viewH, int refreshHz, boolean positionTracking);
    private static native void nativeFeedImu(int type, float x, float y, float z, long tNs);
    private static native boolean nativeFeedPoseText(String text);
    private static native void nativeFeedControllerImu(int hand, float gx, float gy, float gz,
            float ax, float ay, float az, long tNs);
    private static native void nativeFeedControllerInput(int hand, int buttons,
            float ax, float ay, float trigger, float grip);
    private static native void nativeSetControllerConnected(int hand, boolean connected);
    private static native void nativeGetHeadPose(float[] out7);
    private static native boolean nativeHeadPosValid();
    private static native void nativeRecenter();
    private static native String[] nativeDecoderCandidates();
    // HID 眼镜状态（协议应答，去 VID/PID）
    private static native boolean nativeFeedHidInput(byte[] data);
    private static native void nativeSetDeviceStatus(int batteryPercent, boolean proximityNear,
            boolean validBat, boolean validProx);
    private static native void nativeGetDeviceStatus(int[] out7);
    // 左右手柄状态 [connected, dof, battery_pct]，供指示灯显示
    private static native void nativeGetControllerStatus(int hand, int[] out3);
    private static native void nativeSetControllerBattery(int hand, int batteryPct);

    private SensorManager sensorManager;
    private SurfaceView surfaceView;
    private TextView statusText;
    // 三个设备指示灯（头显 / 左手柄 / 右手柄）
    private TextView ledHeadset, ledLeft, ledRight;
    private final Handler uiHandler = new Handler(Looper.getMainLooper());
    private final Runnable ledRefresh = new Runnable() {
        @Override public void run() {
            refreshDeviceLeds();
            uiHandler.postDelayed(this, 1000);
        }
    };
    private Thread udpThread;
    private volatile boolean running = false;

    // 配置（与 ClientConfig 默认一致）
    private static final int VIEW_W = 1600, VIEW_H = 1600, REFRESH_HZ = 72;
    private static final int POSE_UDP_PORT = 47110;

    // 按键位掩码（与 controller.h 一致）
    private static final int BTN_TRIGGER = 1 << 0, BTN_GRIP = 1 << 1, BTN_MENU = 1 << 2;
    private static final int BTN_A = 1 << 4, BTN_B = 1 << 5;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);

        surfaceView = new SurfaceView(this);
        surfaceView.getHolder().addCallback(this);
        statusText = new TextView(this);
        statusText.setTextColor(0xFF00FF00);
        statusText.setTextSize(14);

        // 左上角：三个设备指示灯行（● 点亮绿 / ○ 灰暗）
        ledHeadset = new TextView(this);
        ledLeft    = new TextView(this);
        ledRight   = new TextView(this);
        TextView[] leds = { ledHeadset, ledLeft, ledRight };
        for (TextView tv : leds) {
            tv.setTextSize(16);
            tv.setTextColor(0xFF888888);
            tv.setShadowLayer(3f, 1f, 1f, 0xFF000000);
        }
        LinearLayout ledBox = new LinearLayout(this);
        ledBox.setOrientation(LinearLayout.VERTICAL);
        ledBox.setBackgroundColor(0x99000000);
        ledBox.setPadding(12, 8, 12, 8);
        for (TextView tv : leds) ledBox.addView(tv);

        FrameLayout root = new FrameLayout(this);
        root.addView(surfaceView, new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT));
        FrameLayout.LayoutParams boxLp = new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT);
        boxLp.leftMargin = 16; boxLp.topMargin = 16;
        root.addView(ledBox, boxLp);
        FrameLayout.LayoutParams stLp = new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT);
        stLp.leftMargin = 16; stLp.topMargin = 150;
        root.addView(statusText, stLp);
        setContentView(root);

        nativeInit(VIEW_W, VIEW_H, REFRESH_HZ, false);

        // 手机 IMU + 接近传感器
        sensorManager = (SensorManager) getSystemService(Context.SENSOR_SERVICE);
        registerImu();

        // 电量（CV10 无独立电池，取手机）
        int bat0 = readBatteryPercent();
        nativeSetDeviceStatus(bat0, false, bat0 >= 0, false);

        // 高通解码器探测
        String decoder = pickQualcommDecoder();
        statusText.setText(buildStatus(decoder));

        // UDP 外部姿态注入监听
        running = true;
        udpThread = new Thread(this::udpLoop, "pose-udp");
        udpThread.start();

        // 触摸虚拟手柄（左右半屏）
        surfaceView.setOnTouchListener(this::onTouch);

        // 手柄电量近似取手机电量（蓝牙手柄多数不单独上报电量）
        int bat = bat0;
        nativeSetControllerBattery(0, bat);
        nativeSetControllerBattery(1, bat);

        // 每 1 秒刷新三个设备指示灯
        uiHandler.post(ledRefresh);
    }

    /** 刷新头显/左手柄/右手柄三个指示灯：在线绿色点亮，离线灰暗。 */
    private void refreshDeviceLeds() {
        // 头显：插头/佩戴任一成立即视为在线；电量取设备状态
        int[] dev = new int[7];
        nativeGetDeviceStatus(dev);
        boolean headOnline = dev[4] != 0 || dev[0] != 0; // headset_plugged 或 proximity_near
        int headBat = dev[2] >= 0 ? dev[2] : -1;
        setLed(ledHeadset, "头显 VR Glass", headOnline, headBat, 0);

        // 左右手柄
        int[] c = new int[3];
        nativeGetControllerStatus(0, c);
        setLed(ledLeft, "左手柄", c[0] != 0, c[2], c[1]);
        nativeGetControllerStatus(1, c);
        setLed(ledRight, "右手柄", c[0] != 0, c[2], c[1]);
    }

    /** 用圆点 + 文字渲染一个指示灯。 */
    private void setLed(TextView tv, String name, boolean online, int battery, int dof) {
        StringBuilder s = new StringBuilder();
        s.append(online ? "● " : "○ ").append(name);
        if (online) {
            s.append(" · 已连接");
            if (dof > 0) s.append(" · ").append(dof).append("DoF");
            if (battery >= 0) s.append(" · ").append(battery).append("%");
        } else {
            s.append(" · 未连接");
        }
        tv.setText(s.toString());
        tv.setTextColor(online ? Color.parseColor("#4ADE80") : Color.parseColor("#888888"));
    }

    private void registerImu() {
        if (sensorManager == null) return;
        Sensor gyro = sensorManager.getDefaultSensor(Sensor.TYPE_GYROSCOPE);
        Sensor acc = sensorManager.getDefaultSensor(Sensor.TYPE_ACCELEROMETER);
        Sensor rot = sensorManager.getDefaultSensor(Sensor.TYPE_ROTATION_VECTOR);
        if (gyro != null) sensorManager.registerListener(this, gyro, SensorManager.SENSOR_DELAY_FASTEST);
        if (acc != null) sensorManager.registerListener(this, acc, SensorManager.SENSOR_DELAY_FASTEST);
        // 855 Plus 支持 1000Hz IMU；ROTATION_VECTOR 作为融合四元数直注源
        if (rot != null && gyro == null) sensorManager.registerListener(this, rot, SensorManager.SENSOR_DELAY_FASTEST);
        // 接近传感器：CV10 HID 无 report 4，用 TYPE_PROXIMITY 等价（佩戴检测）
        Sensor prox = sensorManager.getDefaultSensor(Sensor.TYPE_PROXIMITY);
        if (prox != null) sensorManager.registerListener(this, prox, SensorManager.SENSOR_DELAY_NORMAL);
    }

    @Override
    public void onSensorChanged(SensorEvent e) {
        long tNs = e.timestamp;
        int type = -1;
        if (e.sensor.getType() == Sensor.TYPE_GYROSCOPE) type = 0;
        else if (e.sensor.getType() == Sensor.TYPE_ACCELEROMETER) type = 1;
        else if (e.sensor.getType() == Sensor.TYPE_PROXIMITY) {
            // near=贴近(读数<量程,常为0), far=远离(=maxRange)
            boolean near = e.values[0] < e.sensor.getMaximumRange();
            int battery = readBatteryPercent();
            nativeSetDeviceStatus(battery, near, battery >= 0, true);
            return;
        }
        else if (e.sensor.getType() == Sensor.TYPE_ROTATION_VECTOR) {
            // 融合四元数直接注入（x,y,z,w）
            float[] q = new float[4];
            SensorManager.getQuaternionFromVector(q, e.values);
            String text = String.format(Locale.US, "%.6f %.6f %.6f %.6f", q[3], q[0], q[1], q[2]);
            nativeFeedPoseText(text);
            return;
        }
        if (type >= 0) nativeFeedImu(type, e.values[0], e.values[1], e.values[2], tNs);
    }

    @Override public void onAccuracyChanged(Sensor s, int a) { }

    /** 眼镜由手机 USB-C 供电，无独立电池 → 取手机电量（与 compat 仓库 VrAndroidSensors 一致）。 */
    private int readBatteryPercent() {
        BatteryManager bm = (BatteryManager) getSystemService(Context.BATTERY_SERVICE);
        if (bm == null) return -1;
        return bm.getIntProperty(BatteryManager.BATTERY_PROPERTY_CAPACITY);
    }

    /** 从 MediaCodecList 中挑高通组件，绝不选软解。 */
    private String pickQualcommDecoder() {
        String[] candidates = nativeDecoderCandidates();
        MediaCodecList list = new MediaCodecList(MediaCodecList.ALL_CODECS);
        MediaCodecInfo[] infos = list.getCodecInfos();
        for (String want : candidates) {
            for (MediaCodecInfo info : infos) {
                if (info.isEncoder()) continue;
                if (want.equalsIgnoreCase(info.getName())) return want;
            }
        }
        return "NONE(禁止软解：未找到高通硬解组件)";
    }

    private String buildStatus(String decoder) {
        return "通用 ALVR 接收端（无华为依赖）\n"
                + "视图: " + VIEW_W + "x" + VIEW_H + " SBS " + (VIEW_W * 2) + "x" + VIEW_H + "\n"
                + "姿态源: 手机IMU + UDP:" + POSE_UDP_PORT + "\n"
                + "手柄: 3DoF/6DoF 双模\n"
                + "解码器: " + decoder;
    }

    private void udpLoop() {
        try (DatagramSocket sock = new DatagramSocket(POSE_UDP_PORT)) {
            byte[] buf = new byte[256];
            while (running) {
                DatagramPacket p = new DatagramPacket(buf, buf.length);
                sock.receive(p);
                String text = new String(p.getData(), p.getOffset(), p.getLength());
                nativeFeedPoseText(text.trim());
            }
        } catch (Exception ignored) { }
    }

    private boolean onTouch(View v, MotionEvent e) {
        int hand = e.getX() < surfaceView.getWidth() / 2f ? 0 : 1;
        float ax = (e.getX() / (float) surfaceView.getWidth() - (hand == 0 ? 0f : 0.5f)) * 2f;
        float ay = e.getY() / (float) surfaceView.getHeight() * 2f - 1f;
        int buttons = 0;
        float trigger = 0, grip = 0;
        switch (e.getActionMasked()) {
            case MotionEvent.ACTION_DOWN: buttons |= BTN_TRIGGER; trigger = 1; break;
            case MotionEvent.ACTION_MOVE: trigger = 1; break;
            case MotionEvent.ACTION_UP: default: break;
        }
        nativeSetControllerConnected(hand, true);
        nativeFeedControllerInput(hand, buttons, ax, ay, trigger, grip);
        return true;
    }

    // ---- SurfaceHolder.Callback：把 ANativeWindow 交给 native/OpenXR ----
    @Override public void surfaceCreated(SurfaceHolder h) { }
    @Override public void surfaceChanged(SurfaceHolder h, int f, int w, int ht) { }
    @Override public void surfaceDestroyed(SurfaceHolder h) { }

    public void recenter(View v) { nativeRecenter(); }

    @Override protected void onDestroy() {
        running = false;
        uiHandler.removeCallbacks(ledRefresh);
        if (udpThread != null) udpThread.interrupt();
        if (sensorManager != null) sensorManager.unregisterListener(this);
        super.onDestroy();
    }
}
